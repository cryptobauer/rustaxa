#include <array>
#include <cstdint>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <optional>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

#if defined(RUSTAXA_ENABLE_STORAGE)
#include "rustaxa-bridge/ffi.rs.h"
#else
#include <libdevcrypto/Common.h>

#include "storage/storage.hpp"
#include "transaction/system_transaction.hpp"
#endif

namespace taraxa::storage_conformance {
namespace fs = std::filesystem;

namespace {
struct TranscriptEntry {
  std::string key;
  std::string value;
};

class Transcript {
 public:
  void add(std::string key, std::string value) { entries_.push_back({std::move(key), std::move(value)}); }

  std::string toJson() const {
    std::ostringstream out;
    out << "{\n  \"entries\": [\n";
    for (size_t i = 0; i < entries_.size(); ++i) {
      out << "    {\"key\": \"" << escape(entries_[i].key) << "\", \"value\": \"" << escape(entries_[i].value) << "\"}";
      if (i + 1 != entries_.size()) {
        out << ",";
      }
      out << "\n";
    }
    out << "  ]\n}\n";
    return out.str();
  }

 private:
  static std::string escape(const std::string& in) {
    std::ostringstream out;
    for (char c : in) {
      switch (c) {
        case '\\':
          out << "\\\\";
          break;
        case '"':
          out << "\\\"";
          break;
        case '\n':
          out << "\\n";
          break;
        case '\r':
          out << "\\r";
          break;
        case '\t':
          out << "\\t";
          break;
        default:
          out << c;
          break;
      }
    }
    return out.str();
  }

  std::vector<TranscriptEntry> entries_;
};

class TempDir {
 public:
  TempDir() {
    auto const path = fs::temp_directory_path() / "taraxa_storage_conformance";
    std::error_code ec;
    fs::remove_all(path, ec);
    fs::create_directories(path);
    path_ = path;
  }

  ~TempDir() {
    std::error_code ec;
    fs::remove_all(path_, ec);
  }

  const fs::path& path() const { return path_; }

 private:
  fs::path path_;
};

std::string toString(bool value) { return value ? "true" : "false"; }

template <typename T>
std::string toString(T value) {
  return std::to_string(value);
}

template <typename T>
std::string optionalToString(const std::optional<T>& value) {
  if (!value) {
    return "none";
  }
  return toString(*value);
}

#if defined(RUSTAXA_ENABLE_STORAGE)
std::optional<uint64_t> leToU64(const std::vector<uint8_t>& bytes) {
  if (bytes.size() != 8) {
    return std::nullopt;
  }
  uint64_t v = 0;
  for (size_t i = 0; i < bytes.size(); ++i) {
    v |= static_cast<uint64_t>(bytes[i]) << (8 * i);
  }
  return v;
}

rust::Vec<uint8_t> toRustVec(const std::vector<uint8_t>& bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (auto b : bytes) {
    out.push_back(b);
  }
  return out;
}

std::array<uint8_t, 32> h256Array(uint8_t last_byte) {
  std::array<uint8_t, 32> out{};
  out[31] = last_byte;
  return out;
}

std::vector<uint8_t> encodeSingleHashListRlp(const std::array<uint8_t, 32>& hash) {
  // RLP([hash32]) = 0xE1 0xA0 <32-bytes>
  std::vector<uint8_t> out;
  out.reserve(34);
  out.push_back(0xE1);
  out.push_back(0xA0);
  out.insert(out.end(), hash.begin(), hash.end());
  return out;
}

std::optional<uint64_t> toOptional(const rustaxa::PeriodLookup& lookup) {
  if (!lookup.found) {
    return std::nullopt;
  }
  return lookup.period;
}

std::optional<uint32_t> toOptional(const rustaxa::PeriodLambda& value) {
  if (!value.found) {
    return std::nullopt;
  }
  return value.value;
}

void runConformance(const fs::path& db_path, Transcript& transcript) {
  static constexpr uint8_t kStatusFieldExecutedBlkCount = 0;
  static constexpr uint8_t kStatusFieldTrxCount = 2;
  static constexpr uint8_t kStatusFieldDagBlkCount = 3;
  static constexpr uint8_t kStatusFieldDagEdgeCount = 4;
  static constexpr uint8_t kPbftMgrFieldRound = 0;
  static constexpr uint8_t kPbftMgrStatusExecutedBlock = 0;
  static constexpr uint8_t kPbftMgrStatusNextVotedSoftValue = 2;

  auto storage = rustaxa::create_storage(db_path.string());
  auto dag_queries = rustaxa::create_dag_storage_queries(*storage);
  auto metadata_queries = rustaxa::create_metadata_storage_queries(*storage);
  auto pbft_queries = rustaxa::create_pbft_storage_queries(*storage);
  auto final_chain_queries = rustaxa::create_final_chain_storage_queries(*storage);
  auto transaction_queries = rustaxa::create_transaction_storage_queries(*storage);
  auto period_queries = rustaxa::create_period_storage_queries(*storage);

  // Baseline API coverage
  transcript.add("status_default_executed_blk",
                 toString(metadata_queries->get_status_field(kStatusFieldExecutedBlkCount)));
  transcript.add("pbft_mgr_field_default_round", toString(pbft_queries->get_pbft_mgr_field(kPbftMgrFieldRound)));
  transcript.add("pbft_mgr_status_default_executed_block",
                 toString(pbft_queries->get_pbft_mgr_status(kPbftMgrStatusExecutedBlock)));
  transcript.add("proposal_period_missing",
                 optionalToString(toOptional(dag_queries->get_proposal_period_for_dag_level(100))));
  transcript.add("period_lambda_missing", optionalToString(toOptional(metadata_queries->get_period_lambda(7, false))));
  transcript.add("rounds_count_dynamic_lambda_default", toString(metadata_queries->get_rounds_count_dynamic_lambda()));
  transcript.add("genesis_missing_before", toString(metadata_queries->get_genesis_hash().empty()));

  auto genesis_hash = h256Array(0xAB);
  storage->set_genesis_hash(genesis_hash);
  transcript.add("genesis_after_set_len", toString(metadata_queries->get_genesis_hash().size()));

  storage->save_status_field(kStatusFieldTrxCount, 11);
  storage->save_pbft_mgr_field(kPbftMgrFieldRound, 17);
  storage->save_pbft_mgr_status(kPbftMgrStatusNextVotedSoftValue, true);
  storage->save_proposal_period_dag_levels_map(100, 50);
  storage->save_period_lambda(7, 42);
  storage->save_rounds_count_dynamic_lambda(23);

  transcript.add("status_trx_count_after_save", toString(metadata_queries->get_status_field(kStatusFieldTrxCount)));
  transcript.add("pbft_mgr_field_round_after_save", toString(pbft_queries->get_pbft_mgr_field(kPbftMgrFieldRound)));
  transcript.add("pbft_mgr_status_next_voted_soft_after_save",
                 toString(pbft_queries->get_pbft_mgr_status(kPbftMgrStatusNextVotedSoftValue)));
  transcript.add("proposal_period_level_100_after_save",
                 optionalToString(toOptional(dag_queries->get_proposal_period_for_dag_level(100))));
  transcript.add("period_lambda_exact_after_save",
                 optionalToString(toOptional(metadata_queries->get_period_lambda(7, false))));
  transcript.add("period_lambda_closest_after_save",
                 optionalToString(toOptional(metadata_queries->get_period_lambda(8, true))));
  transcript.add("rounds_count_dynamic_lambda_after_save",
                 toString(metadata_queries->get_rounds_count_dynamic_lambda()));

  // DAG missing + save/update/remove paths
  auto dag_hash_1 = h256Array(0x11);
  auto dag_hash_2 = h256Array(0x22);
  auto dag_missing = h256Array(0xEE);
  auto dag_rlp = std::vector<uint8_t>{0xC0};

  transcript.add("dag_missing_block", toString(dag_queries->get_dag_block(dag_missing).empty()));
  transcript.add("dag_missing_period", toString(!dag_queries->get_dag_block_period_lookup(dag_missing).found));

  storage->save_dag_block(dag_hash_1, 1, 0, toRustVec(dag_rlp));
  storage->save_dag_block(dag_hash_2, 1, 1, toRustVec(dag_rlp));
  transcript.add("dag_saved_primary", toString(dag_queries->dag_block_in_db(dag_hash_1)));
  transcript.add("dag_saved_batch", toString(dag_queries->dag_block_in_db(dag_hash_2)));
  transcript.add("dag_level_1_count", toString(dag_queries->get_blocks_by_level(1).size() / 32));

  storage->save_dag_block_period(dag_hash_1, 7, 2);
  auto dag_period = dag_queries->get_dag_block_period_lookup(dag_hash_1);
  transcript.add("dag_period_lookup_found", toString(dag_period.found));
  transcript.add("dag_period_lookup_period", toString(dag_period.period));
  transcript.add("dag_period_lookup_position", toString(dag_period.position));

  transcript.add("dag_counters_nonzero", toString(metadata_queries->get_status_field(kStatusFieldDagBlkCount) > 0 &&
                                                  metadata_queries->get_status_field(kStatusFieldDagEdgeCount) > 0));

  storage->remove_dag_block(dag_hash_2);
  transcript.add("dag_removed_batch_hash", toString(!dag_queries->dag_block_in_db(dag_hash_2)));
  transcript.add("dag_last_level", toString(dag_queries->get_last_blocks_level()));
  transcript.add("dag_blocks_at_level_span_count", toString(dag_queries->get_dag_blocks_at_level(1, 2).size()));

  // Period by PBFT hash mapping
  auto pbft_hash = h256Array(0x44);
  auto pbft_missing = h256Array(0x45);
  storage->save_pbft_block_period(pbft_hash, 99);
  auto pbft_lookup = period_queries->get_period_from_pbft_hash(pbft_hash);
  auto pbft_missing_lookup = period_queries->get_period_from_pbft_hash(pbft_missing);
  transcript.add("pbft_period_lookup_found", toString(pbft_lookup.found));
  transcript.add("pbft_period_lookup_value", toString(pbft_lookup.period));
  transcript.add("pbft_period_lookup_missing", toString(!pbft_missing_lookup.found));
  transcript.add("pbft_block_in_db_found", toString(pbft_queries->pbft_block_in_db(pbft_hash)));
  transcript.add("pbft_block_in_db_missing", toString(pbft_queries->pbft_block_in_db(pbft_missing)));

  auto pbft_head_hash = h256Array(0x71);
  transcript.add("pbft_head_missing_len", toString(pbft_queries->get_pbft_head(pbft_head_hash).size()));
  storage->save_pbft_head(pbft_head_hash, toRustVec(std::vector<uint8_t>{'h', 'e', 'a', 'd'}));
  transcript.add("pbft_head_after_save_len", toString(pbft_queries->get_pbft_head(pbft_head_hash).size()));

  // Transaction paths + system transaction + period system hashes
  auto tx_hash_1 = h256Array(0x51);
  auto tx_hash_2 = h256Array(0x52);
  auto sys_hash = h256Array(0x53);
  auto tx_rlp = std::vector<uint8_t>{0xC0};

  storage->save_transaction(tx_hash_1, toRustVec(tx_rlp));
  storage->save_transaction(tx_hash_2, toRustVec(tx_rlp));

  transcript.add("tx_hash_1_in_db", toString(transaction_queries->transaction_in_db(tx_hash_1)));
  transcript.add("tx_hash_1_finalized_before", toString(transaction_queries->transaction_finalized(tx_hash_1)));

  storage->save_transaction_location(tx_hash_1, 12, 0, false);
  transcript.add("tx_hash_1_finalized_after", toString(transaction_queries->transaction_finalized(tx_hash_1)));
  transcript.add("tx_hash_1_location_present",
                 toString(!transaction_queries->get_transaction_location(tx_hash_1).empty()));
  transcript.add("tx_hash_1_lookup_nonempty", toString(!transaction_queries->get_transaction(tx_hash_1).empty()));
  transcript.add("tx_period_map_size", toString(transaction_queries->get_all_transaction_period().size()));

  storage->remove_transaction(tx_hash_2);
  transcript.add("tx_hash_2_removed", toString(!transaction_queries->transaction_in_db(tx_hash_2)));
  transcript.add("tx_nonfinalized_count", toString(transaction_queries->get_all_nonfinalized_transactions().size()));
  std::string tx_finalized_vector;
  tx_finalized_vector.push_back(transaction_queries->transaction_finalized(tx_hash_1) ? '1' : '0');
  tx_finalized_vector.push_back(transaction_queries->transaction_finalized(tx_hash_2) ? '1' : '0');
  transcript.add("tx_finalized_vector", tx_finalized_vector);

  storage->save_system_transaction(sys_hash, toRustVec(tx_rlp));
  transcript.add("system_tx_lookup_nonempty", toString(!transaction_queries->get_system_transaction(sys_hash).empty()));

  storage->save_period_system_transactions_hashes(12, toRustVec(encodeSingleHashListRlp(sys_hash)));
  auto period_sys_hashes = transaction_queries->get_period_system_transactions_hashes(12);
  transcript.add("period_system_hashes_count", toString(period_sys_hashes.size() / 32));

  auto period_data_raw = std::vector<uint8_t>{0xC6, 0xC0, 0xC0, 0xC0, 0xE1, 0xC0, 0xC0};
  storage->save_period_data(33, toRustVec(period_data_raw));
  transcript.add("period_data_raw_len", toString(period_queries->get_period_data_raw(33).size()));

  // Final-chain lookup/intercepted columns
  uint32_t const meta_key = 99;
  uint64_t const block_number = 42;
  auto block_hash = h256Array(0x61);
  auto receipt_hash = h256Array(0x62);
  auto blooms_chunk = h256Array(0x63);

  auto meta_value = std::vector<uint8_t>{'m', 'e', 't', 'a'};
  auto block_value = std::vector<uint8_t>{'b', 'l', 'k'};
  auto receipt_value = std::vector<uint8_t>{'r', 'c', 'p'};
  auto blooms_value = std::vector<uint8_t>{'b', 'l', 'm'};

  storage->seed_final_chain_conformance_lookup_rows(
      meta_key, toRustVec(meta_value), block_number, block_hash, toRustVec(block_value), receipt_hash,
      toRustVec(receipt_value), blooms_chunk, toRustVec(blooms_value), 15, toRustVec(std::vector<uint8_t>{0xC0}));

  auto meta_lookup = final_chain_queries->get_final_chain_meta_value(meta_key);
  auto block_lookup = final_chain_queries->get_final_chain_block_header(block_number);
  auto hash_lookup = final_chain_queries->get_final_chain_block_hash_by_number(block_number);
  auto number_lookup_raw = final_chain_queries->get_final_chain_block_number_by_hash(block_hash);
  auto receipt_lookup = final_chain_queries->get_final_chain_receipt_by_trx_hash(receipt_hash);
  auto blooms_lookup = final_chain_queries->get_final_chain_log_blooms_chunk(blooms_chunk);
  auto receipt_by_period_raw = period_queries->get_block_receipt(15);

  transcript.add("final_chain_meta_len", toString(meta_lookup.size()));
  transcript.add("final_chain_block_len", toString(block_lookup.size()));
  transcript.add("final_chain_hash_len", toString(hash_lookup.size()));
  transcript.add("final_chain_number_by_hash",
                 optionalToString(leToU64(std::vector<uint8_t>(number_lookup_raw.begin(), number_lookup_raw.end()))));
  transcript.add("final_chain_receipt_len", toString(receipt_lookup.size()));
  transcript.add("final_chain_blooms_len", toString(blooms_lookup.size()));
  transcript.add("final_chain_receipts_by_period_count", toString(receipt_by_period_raw.size()));
}
#else
void runConformance(const fs::path& db_path, Transcript& transcript) {
  DbStorage storage(db_path);

  transcript.add("status_default_executed_blk", toString(storage.getStatusField(StatusDbField::ExecutedBlkCount)));
  transcript.add("pbft_mgr_field_default_round", toString(storage.getPbftMgrField(PbftMgrField::Round)));
  transcript.add("pbft_mgr_status_default_executed_block",
                 toString(storage.getPbftMgrStatus(PbftMgrStatus::ExecutedBlock)));
  transcript.add("proposal_period_missing", optionalToString(storage.getProposalPeriodForDagLevel(100)));
  transcript.add("period_lambda_missing", optionalToString(storage.getPeriodLambda(7, false)));
  transcript.add("rounds_count_dynamic_lambda_default", toString(storage.getRoundsCountDynamicLambda()));
  transcript.add("genesis_missing_before", toString(!storage.getGenesisHash().has_value()));

  storage.setGenesisHash(h256(0xAB));
  auto genesis_after_set = storage.getGenesisHash();
  transcript.add("genesis_after_set_len", toString(genesis_after_set ? genesis_after_set->asBytes().size() : 0));

  storage.saveStatusField(StatusDbField::TrxCount, 11);
  storage.savePbftMgrField(PbftMgrField::Round, 17);
  storage.savePbftMgrStatus(PbftMgrStatus::NextVotedSoftValue, true);
  storage.saveProposalPeriodDagLevelsMap(100, 50);
  {
    auto batch = DbStorage::createWriteBatch();
    storage.savePeriodLambda(7, 42, batch);
    storage.saveRoundsCountDynamicLambda(23, batch);
    storage.commitWriteBatch(batch);
  }

  transcript.add("status_trx_count_after_save", toString(storage.getStatusField(StatusDbField::TrxCount)));
  transcript.add("pbft_mgr_field_round_after_save", toString(storage.getPbftMgrField(PbftMgrField::Round)));
  transcript.add("pbft_mgr_status_next_voted_soft_after_save",
                 toString(storage.getPbftMgrStatus(PbftMgrStatus::NextVotedSoftValue)));
  transcript.add("proposal_period_level_100_after_save", optionalToString(storage.getProposalPeriodForDagLevel(100)));
  transcript.add("period_lambda_exact_after_save", optionalToString(storage.getPeriodLambda(7, false)));
  transcript.add("period_lambda_closest_after_save", optionalToString(storage.getPeriodLambda(8, true)));
  transcript.add("rounds_count_dynamic_lambda_after_save", toString(storage.getRoundsCountDynamicLambda()));

  // DAG missing + save/update/remove paths
  auto dag_1 = std::make_shared<DagBlock>(blk_hash_t(0x11), 1, vec_blk_t{}, vec_trx_t{}, sig_t(11), blk_hash_t(0xA1),
                                          addr_t(0x1));
  auto dag_2 = std::make_shared<DagBlock>(blk_hash_t(0x22), 1, vec_blk_t{dag_1->getHash()}, vec_trx_t{}, sig_t(22),
                                          blk_hash_t(0xA2), addr_t(0x2));
  auto dag_3 = std::make_shared<DagBlock>(blk_hash_t(0x33), 2, vec_blk_t{dag_1->getHash(), dag_2->getHash()},
                                          vec_trx_t{}, sig_t(33), blk_hash_t(0xA3), addr_t(0x3));

  transcript.add("dag_missing_block", toString(storage.getDagBlock(blk_hash_t(0xEE)) == nullptr));
  transcript.add("dag_missing_period", toString(storage.getDagBlockPeriod(blk_hash_t(0xEE)) == nullptr));

  storage.saveDagBlock(dag_1);
  {
    auto batch = DbStorage::createWriteBatch();
    storage.saveDagBlock(dag_2, &batch);
    storage.commitWriteBatch(batch);
  }

  transcript.add("dag_saved_primary", toString(storage.dagBlockInDb(dag_1->getHash())));
  transcript.add("dag_saved_batch", toString(storage.dagBlockInDb(dag_2->getHash())));
  transcript.add("dag_level_1_count", toString(storage.getBlocksByLevel(1).size()));

  {
    auto batch = DbStorage::createWriteBatch();
    storage.addDagBlockPeriodToBatch(dag_1->getHash(), 7, 2, batch);
    storage.commitWriteBatch(batch);
  }
  auto dag_period = storage.getDagBlockPeriod(dag_1->getHash());
  transcript.add("dag_period_lookup_found", toString(dag_period != nullptr));
  transcript.add("dag_period_lookup_period", toString(dag_period ? dag_period->first : 0));
  transcript.add("dag_period_lookup_position", toString(dag_period ? dag_period->second : 0));

  storage.updateDagBlockCounters({dag_3});
  transcript.add("dag_counters_nonzero", toString(storage.getStatusField(StatusDbField::DagBlkCount) > 0 &&
                                                  storage.getStatusField(StatusDbField::DagEdgeCount) > 0));

  storage.removeDagBlock(dag_2->getHash());
  transcript.add("dag_removed_batch_hash", toString(!storage.dagBlockInDb(dag_2->getHash())));
  transcript.add("dag_last_level", toString(storage.getLastBlocksLevel()));
  transcript.add("dag_blocks_at_level_span_count", toString(storage.getDagBlocksAtLevel(1, 2).size()));

  // Period by PBFT hash mapping
  {
    auto batch = DbStorage::createWriteBatch();
    storage.addPbftBlockPeriodToBatch(99, blk_hash_t(0x44), batch);
    storage.commitWriteBatch(batch);
  }
  auto pbft_lookup = storage.getPeriodFromPbftHash(blk_hash_t(0x44));
  auto pbft_missing_lookup = storage.getPeriodFromPbftHash(blk_hash_t(0x45));
  transcript.add("pbft_period_lookup_found", toString(pbft_lookup.first));
  transcript.add("pbft_period_lookup_value", toString(pbft_lookup.second));
  transcript.add("pbft_period_lookup_missing", toString(!pbft_missing_lookup.first));
  transcript.add("pbft_block_in_db_found", toString(storage.pbftBlockInDb(blk_hash_t(0x44))));
  transcript.add("pbft_block_in_db_missing", toString(storage.pbftBlockInDb(blk_hash_t(0x45))));

  transcript.add("pbft_head_missing_len", toString(storage.getPbftHead(blk_hash_t(0x71)).size()));
  storage.savePbftHead(blk_hash_t(0x71), "head");
  transcript.add("pbft_head_after_save_len", toString(storage.getPbftHead(blk_hash_t(0x71)).size()));

  // Transaction paths + system transaction + period system hashes
  auto sk = secret_t::random();
  auto tx_1 = std::make_shared<Transaction>(1, 1, 1, 21000, bytes{}, sk, addr_t(0x11));
  auto tx_2 = std::make_shared<Transaction>(2, 1, 1, 21000, bytes{}, sk, addr_t(0x12));

  {
    auto batch = DbStorage::createWriteBatch();
    storage.addTransactionToBatch(*tx_1, batch);
    storage.addTransactionToBatch(*tx_2, batch);
    storage.commitWriteBatch(batch);
  }

  transcript.add("tx_hash_1_in_db", toString(storage.transactionInDb(tx_1->getHash())));
  transcript.add("tx_hash_1_finalized_before", toString(storage.transactionFinalized(tx_1->getHash())));

  {
    auto batch = DbStorage::createWriteBatch();
    storage.addTransactionLocationToBatch(batch, tx_1->getHash(), 12, 0, false);
    storage.commitWriteBatch(batch);
  }

  transcript.add("tx_hash_1_finalized_after", toString(storage.transactionFinalized(tx_1->getHash())));
  transcript.add("tx_hash_1_location_present", toString(storage.getTransactionLocation(tx_1->getHash()).has_value()));
  transcript.add("tx_hash_1_lookup_nonempty", toString(storage.getTransaction(tx_1->getHash()) != nullptr));
  transcript.add("tx_period_map_size", toString(storage.getAllTransactionPeriod().size()));

  {
    auto batch = DbStorage::createWriteBatch();
    storage.removeTransactionToBatch(tx_2->getHash(), batch);
    storage.commitWriteBatch(batch);
  }
  transcript.add("tx_hash_2_removed", toString(!storage.transactionInDb(tx_2->getHash())));
  transcript.add("tx_nonfinalized_count", toString(storage.getAllNonfinalizedTransactions().size()));
  auto tx_finalized = storage.transactionsFinalized({tx_1->getHash(), tx_2->getHash()});
  std::string tx_finalized_vector;
  tx_finalized_vector.push_back(tx_finalized.size() > 0 && tx_finalized[0] ? '1' : '0');
  tx_finalized_vector.push_back(tx_finalized.size() > 1 && tx_finalized[1] ? '1' : '0');
  transcript.add("tx_finalized_vector", tx_finalized_vector);

  auto sys_tx = std::make_shared<SystemTransaction>(1, 1, 1, 21000, bytes{}, addr_t(0x33));
  {
    auto batch = DbStorage::createWriteBatch();
    storage.addSystemTransactionToBatch(batch, sys_tx);
    storage.addPeriodSystemTransactions(batch, {sys_tx}, 12);
    storage.commitWriteBatch(batch);
  }

  transcript.add("system_tx_lookup_nonempty", toString(storage.getSystemTransaction(sys_tx->getHash()) != nullptr));
  transcript.add("period_system_hashes_count", toString(storage.getPeriodSystemTransactionsHashes(12).size()));

  {
    auto batch = DbStorage::createWriteBatch();
    storage.insert(batch, DbStorage::Columns::period_data, static_cast<uint64_t>(33),
                   bytes{0xC6, 0xC0, 0xC0, 0xC0, 0xE1, 0xC0, 0xC0});
    storage.commitWriteBatch(batch);
  }
  transcript.add("period_data_raw_len", toString(storage.getPeriodDataRaw(33).size()));

  // Final-chain lookup/intercepted columns
  uint32_t const meta_key = 99;
  uint64_t const block_number = 42;
  blk_hash_t const block_hash(0x61);
  trx_hash_t const receipt_hash(0x62);
  h256 const blooms_chunk(0x63);

  std::string const meta_value = "meta";
  std::string const block_value = "blk";
  std::string const receipt_value = "rcp";
  std::string const blooms_value = "blm";
  bytes const empty_receipts_raw{0xC0};

  {
    auto batch = DbStorage::createWriteBatch();
    storage.insert(batch, DbStorage::Columns::final_chain_meta, meta_key, meta_value);
    storage.insert(batch, DbStorage::Columns::final_chain_blk_by_number, block_number, block_value);
    storage.insert(batch, DbStorage::Columns::final_chain_blk_hash_by_number, block_number, block_hash.asBytes());
    storage.insert(batch, DbStorage::Columns::final_chain_blk_number_by_hash, block_hash.asBytes(), block_number);
    storage.insert(batch, DbStorage::Columns::final_chain_receipt_by_trx_hash, receipt_hash.asBytes(), receipt_value);
    storage.insert(batch, DbStorage::Columns::final_chain_log_blooms_index, blooms_chunk.asBytes(), blooms_value);
    storage.insert(batch, DbStorage::Columns::final_chain_receipt_by_period, static_cast<uint64_t>(15),
                   empty_receipts_raw);
    storage.commitWriteBatch(batch);
  }

  auto meta_lookup = storage.lookup(meta_key, DbStorage::Columns::final_chain_meta);
  auto block_lookup = storage.lookup(block_number, DbStorage::Columns::final_chain_blk_by_number);
  auto hash_lookup = storage.lookup(block_number, DbStorage::Columns::final_chain_blk_hash_by_number);
  auto number_lookup =
      storage.lookup_int<uint64_t>(block_hash.asBytes(), DbStorage::Columns::final_chain_blk_number_by_hash);
  auto receipt_lookup = storage.lookup(receipt_hash.asBytes(), DbStorage::Columns::final_chain_receipt_by_trx_hash);
  auto blooms_lookup = storage.lookup(blooms_chunk.asBytes(), DbStorage::Columns::final_chain_log_blooms_index);
  auto receipt_by_period_raw =
      storage.lookup(static_cast<uint64_t>(15), DbStorage::Columns::final_chain_receipt_by_period);

  transcript.add("final_chain_meta_len", toString(meta_lookup.size()));
  transcript.add("final_chain_block_len", toString(block_lookup.size()));
  transcript.add("final_chain_hash_len", toString(hash_lookup.size()));
  transcript.add("final_chain_number_by_hash", optionalToString(number_lookup));
  transcript.add("final_chain_receipt_len", toString(receipt_lookup.size()));
  transcript.add("final_chain_blooms_len", toString(blooms_lookup.size()));
  transcript.add("final_chain_receipts_by_period_count", toString(receipt_by_period_raw.size()));
}
#endif

}  // namespace
}  // namespace taraxa::storage_conformance

int main(int argc, char** argv) {
  using namespace taraxa::storage_conformance;
  std::optional<std::string> output_file;
  for (int i = 1; i < argc; ++i) {
    std::string arg = argv[i];
    if (arg == "--output" && i + 1 < argc) {
      output_file = argv[++i];
    }
  }

  TempDir temp_dir;
  Transcript transcript;
  runConformance(temp_dir.path(), transcript);

  auto const json = transcript.toJson();
  if (output_file) {
    std::ofstream out(*output_file);
    out << json;
  } else {
    std::cout << json;
  }
  return 0;
}
