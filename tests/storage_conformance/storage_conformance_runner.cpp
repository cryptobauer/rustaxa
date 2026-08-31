#include <array>
#include <cstdint>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <limits>
#include <optional>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

#if defined(RUSTAXA_ENABLE)
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

#if !defined(RUSTAXA_ENABLE)
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
#endif

#if defined(RUSTAXA_ENABLE)
std::array<uint8_t, 32> h256Array(uint8_t last_byte) {
  std::array<uint8_t, 32> out{};
  out[31] = last_byte;
  return out;
}

rust::Vec<uint8_t> u64Be(uint64_t value) {
  rust::Vec<uint8_t> bytes;
  while (value != 0) {
    bytes.push_back(static_cast<uint8_t>(value & 0xff));
    value >>= 8;
  }
  std::reverse(bytes.begin(), bytes.end());
  return bytes;
}

rust::Box<rustaxa::BridgeConsensusApplication> createConformanceApplication(const fs::path& path) {
  rustaxa::SortitionRuntimeConfig sortition{};
  sortition.threshold_upper = 0x100;
  sortition.difficulty_min = 1;
  sortition.difficulty_max = 10;
  sortition.difficulty_stale = 5;
  sortition.lambda_bound = 100;
  sortition.changes_count_for_average = 8;
  sortition.dag_efficiency_target_low = 5'000;
  sortition.dag_efficiency_target_high = 10'000;
  sortition.changing_interval = 10;
  sortition.computation_interval = 5;

  rustaxa::GasPricerConfig gas_pricer{};
  gas_pricer.percentile = 50;

  rustaxa::PbftServiceConfig pbft{};
  pbft.genesis_lambda_ms = 100;
  pbft.cacti_lambda_max_ms = 1'500;
  pbft.cacti_lambda_default_ms = 500;
  pbft.cacti_block = 1;
  pbft.max_exponential_lambda_ms = 60'000;
  pbft.max_steps = 13;
  pbft.deadline_ms = 1'000;
  pbft.polling_interval_ms = 100;
  pbft.pillar_blocks_interval = 10;
  pbft.sync_level_size = 10;
  pbft.committee_size = 1;
  pbft.number_of_proposers = 1;
  pbft.lambda_min_ms = 100;
  pbft.lambda_change_interval = 10;
  pbft.lambda_change_ms = 10;
  pbft.consensus_delay_ms = 400;
  pbft.dpos_blocks_per_year = 1'000;
  pbft.recently_finalized_factor = 2;
  pbft.chain_id = 1;
  pbft.default_pbft_gas_limit = 1'000'000;
  pbft.cornus_activation_period = std::numeric_limits<uint64_t>::max();
  pbft.cornus_pbft_gas_limit = pbft.default_pbft_gas_limit;

  rustaxa::GenesisDposConfig dpos{};
  dpos.eligibility_balance_threshold = u64Be(1'000);
  dpos.vote_eligibility_balance_step = u64Be(1'000);
  dpos.validator_maximum_stake = u64Be(30'000);

  rustaxa::FinalChainRewardsConfig rewards{};
  rewards.phalaenopsis_period = UINT64_MAX;
  rewards.aspen_part_one_period = UINT64_MAX;
  rewards.fix_claim_all_block_num = UINT64_MAX;
  rewards.fix_redelegate_block_num = UINT64_MAX;
  rewards.aspen_part_two_period = UINT64_MAX;
  rewards.cacti_period = UINT64_MAX;

  auto genesis = h256Array(0xAB);
  auto dag_genesis = h256Array(0xAC);
  auto concrete_genesis_root = h256Array(0xAD);
  return rustaxa::create_consensus_application(path.string(), 1, 0, genesis, dag_genesis, 32, 1'000'000, sortition,
                                               rustaxa::TransactionQueueConfig{16}, gas_pricer, 1'000'000,
                                               std::move(pbft), {}, {}, 1'000'000, 0, 0, concrete_genesis_root, {}, {},
                                               {}, std::move(dpos), std::move(rewards));
}

void runConformance(const fs::path& db_path, Transcript& transcript) {
  auto runtime = createConformanceApplication(db_path);
  for (const auto& observation : rustaxa::consensus_application_run_storage_conformance_v1(*runtime)) {
    transcript.add(std::string(observation.key), std::string(observation.value));
  }
}
#else
void runConformance(const fs::path& db_path, Transcript& transcript) {
  DbStorage storage(db_path);

  transcript.add("status_default_executed_blk", toString(storage.getStatusField(StatusDbField::ExecutedBlkCount)));
  transcript.add("pbft_mgr_field_default_round", toString(storage.getPbftMgrField(PbftMgrField::Round)));
  transcript.add("pbft_mgr_status_default_executed_block",
                 toString(storage.getPbftMgrStatus(PbftMgrStatus::ExecutedBlock)));
  transcript.add("proposal_period_missing", optionalToString(storage.getProposalPeriodForDagLevel(1'000'001)));
  transcript.add("period_lambda_missing", optionalToString(storage.getPeriodLambda(7, false)));
  transcript.add("rounds_count_dynamic_lambda_default", toString(storage.getRoundsCountDynamicLambda()));
  storage.setGenesisHash(h256(0xAB));
  auto genesis_after_set = storage.getGenesisHash();
  transcript.add("genesis_present_before", toString(genesis_after_set && genesis_after_set->asBytes().size() == 32));

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
