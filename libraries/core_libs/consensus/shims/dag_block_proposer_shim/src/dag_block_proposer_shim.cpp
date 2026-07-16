#include <algorithm>
#include <chrono>
#include <future>
#include <memory>
#include <optional>
#include <unordered_map>
#include <unordered_set>
#include <utility>

#include "common/util.hpp"
#include "config/config.hpp"
#include "dag/dag_block_proposer.hpp"
#include "dag/dag_manager.hpp"
#include "final_chain/final_chain.hpp"
#include "key_manager/key_manager.hpp"
#include "libdevcore/Common.h"
#include "libdevcrypto/Common.h"
#include "network/network.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/transaction.hpp"
#include "transaction/transaction_manager.hpp"

namespace taraxa {
namespace {

constexpr uint8_t kDagProposerSessionStatusComplete = 1;
constexpr uint8_t kDagProposerSessionStatusInvalidReport = 2;
constexpr uint8_t kDagProposerSessionActionPackTransactions = 1;
constexpr uint8_t kDagProposerSessionActionStartVdf = 2;
constexpr uint8_t kDagProposerSessionActionCancelVdf = 3;
constexpr uint8_t kDagProposerSessionActionStaleProofSleep = 4;
constexpr uint8_t kDagProposerSessionActionBuildBlock = 5;
constexpr uint8_t kDagProposerSessionActionAddBlock = 6;
constexpr uint8_t kDagProposerSessionActionCollectExternalProposalFacts = 7;
constexpr uint32_t kDagProposerReasonMissingProposalPeriod = 1;
constexpr uint32_t kDagProposerReasonVrfKeyMismatch = 3;
constexpr uint32_t kDagProposerReasonZeroDenominator = 6;
constexpr uint32_t kDagProposerReasonFinalizedPeriodNotReady = 9;
constexpr uint32_t kDagProposerReasonPackedTransactionsEmpty = 14;
constexpr uint32_t kDagProposerReasonTransactionPackThrottled = 16;

std::array<uint8_t, 32> to_bridge_hash(const blk_hash_t& hash) { return hash.asArray(); }

rustaxa::DagHash to_bridge_dag_hash(const blk_hash_t& hash) { return rustaxa::DagHash{to_bridge_hash(hash)}; }

blk_hash_t from_bridge_hash(const std::array<uint8_t, 32>& hash) {
  return blk_hash_t(hash.data(), blk_hash_t::ConstructFromPointer);
}

blk_hash_t from_bridge_dag_hash(const rustaxa::DagHash& hash) { return from_bridge_hash(hash.hash); }

std::vector<blk_hash_t> from_bridge_dag_hashes(const rust::Vec<rustaxa::DagHash>& hashes) {
  std::vector<blk_hash_t> out;
  out.reserve(hashes.size());
  for (const auto& hash : hashes) {
    out.emplace_back(from_bridge_dag_hash(hash));
  }
  return out;
}

dev::bytes to_bytes(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

rust::Vec<uint8_t> to_rust_vec(const dev::bytes& bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(static_cast<uint8_t>(byte));
  }
  return out;
}

rustaxa::LegacySortitionParams to_legacy_sortition_params(const rustaxa::SortitionRuntimeParams& params) {
  rustaxa::LegacySortitionParams out;
  out.vrf_threshold_upper = params.threshold_upper;
  out.vdf_difficulty_min = params.difficulty_min;
  out.vdf_difficulty_max = params.difficulty_max;
  out.vdf_difficulty_stale = params.difficulty_stale;
  out.vdf_lambda_bound = params.lambda_bound;
  return out;
}

vdf_sortition::VdfSortition vdf_sortition_from_proof(const rustaxa::VdfSortitionProofResult& proof) {
  rustaxa::VdfSortitionPayload payload;
  payload.vrf_proof = proof.vrf_proof;
  payload.vdf_solution_proof = proof.vdf_proof;
  payload.vdf_solution_output = proof.vdf_output;
  payload.difficulty = proof.difficulty;
  return vdf_sortition::VdfSortition(to_bytes(rustaxa::vdf_sortition_payload_encode(payload)));
}

}  // namespace

using namespace vdf_sortition;

DagBlockProposer::DagBlockProposer(const FullNodeConfig& config, std::shared_ptr<DagManager> dag_mgr,
                                   std::shared_ptr<TransactionManager> trx_mgr,
                                   std::shared_ptr<final_chain::FinalChain> final_chain,
                                   std::shared_ptr<KeyManager> key_manager)
    : executor_(config.wallets.size()),
      total_trx_shards_(std::max(config.genesis.dag.block_proposer.shard, uint16_t(1))),
      dag_mgr_(std::move(dag_mgr)),
      trx_mgr_(std::move(trx_mgr)),
      final_chain_(std::move(final_chain)),
      nodes_dag_proposers_data_(),
      kDagProposeGasLimit(
          std::min(config.propose_dag_gas_limit, config.genesis.getGasLimits(final_chain_->lastBlockNumber()).first)),
      kPbftGasLimit(config.genesis.getGasLimits(final_chain_->lastBlockNumber()).second),
      kDagGasLimit(config.genesis.getGasLimits(final_chain_->lastBlockNumber()).first) {
  (void)key_manager;
  const auto& node_addr = dev::toAddress(config.getFirstWallet().node_secret);
  LOG_OBJECTS_CREATE("DAG_PROPOSER");

  for (const auto& wallet : config.wallets) {
    nodes_dag_proposers_data_.emplace_back(
        std::make_shared<NodeDagProposerData>(wallet, max_num_tries_, total_trx_shards_));
  }
}

bool DagBlockProposer::proposeDagBlock(const std::shared_ptr<NodeDagProposerData>& node_dag_proposer_data) {
  rustaxa::DagProposerSessionBeginInput begin_input;
  begin_input.transaction_pool_size = trx_mgr_->getTransactionPoolSize();
  begin_input.non_finalized_transaction_count = trx_mgr_->getNonfinalizedTrxSize();
  begin_input.max_non_finalized_transactions = kMaxNonFinalizedTransactions;
  begin_input.dag_expiry_level_limit = kDagExpiryLevelLimit;
  begin_input.wallet_vrf_public_key = node_dag_proposer_data->wallet.vrf_pk.asArray();
  begin_input.wallet_vrf_secret = node_dag_proposer_data->wallet.vrf_secret.asArray();
  begin_input.max_non_finalized_dag_blocks = kMaxNonFinalizedDagBlocks;
  begin_input.max_non_finalized_dag_blocks_low_difficulty = kMaxNonFinalizedDagBlocksLowDifficulty;
  begin_input.max_retry_count = node_dag_proposer_data->max_num_tries;
  begin_input.proposal_weight_limit = kDagProposeGasLimit;
  begin_input.total_transaction_shards = total_trx_shards_;
  begin_input.node_transaction_shard = node_dag_proposer_data->trx_shard;
  begin_input.shard_period_interval = kShardProposePeriodInterval;

  const auto proposer_session_id = dag_mgr_->beginProposerSession(std::move(begin_input));
  dev::ScopeGuard abort_session_on_exit([dag_manager = dag_mgr_, proposer_session_id] {
    try {
      dag_manager->abortProposerSession(proposer_session_id);
    } catch (...) {
      // Destructors must not replace an active proposer exception. Normal terminal
      // paths already removed the session, so the idempotent abort is a no-op.
    }
  });
  auto step = dag_mgr_->proposerSessionNext(proposer_session_id);
  auto fail_on_invalid_report = [](const rustaxa::DagProposerSessionStep& plan) {
    if (plan.status == kDagProposerSessionStatusInvalidReport) {
      throw std::runtime_error("Rust DAG proposer session rejected executor report: " + std::string(plan.error_code));
    }
  };
  auto log_terminal_skip = [&node_dag_proposer_data, this](const rustaxa::DagProposerSessionStep& plan) {
    if (plan.reason_code == kDagProposerReasonMissingProposalPeriod) {
      LOG(log_wr_) << "No proposal period for propose_level " << plan.proposal_level << " found";
    } else if (plan.reason_code == kDagProposerReasonFinalizedPeriodNotReady) {
      LOG(log_wr_) << "Last finalized block period " << plan.last_finalized_period << " < propose_period "
                   << plan.proposal_period;
    } else if (plan.reason_code == kDagProposerReasonVrfKeyMismatch) {
      LOG(log_er_) << "VRF public key mismatch for DAG proposer " << node_dag_proposer_data->wallet.node_addr;
    } else if (plan.reason_code == kDagProposerReasonZeroDenominator) {
      LOG(log_er_) << node_dag_proposer_data->wallet.node_addr
                   << " total vote count 0 at proposal period: " << plan.proposal_period;
    } else if (plan.reason_code == kDagProposerReasonPackedTransactionsEmpty) {
      LOG(log_tr_) << "Skip block proposer, zero sharded transactions ..." << std::endl;
    } else if (plan.reason_code == kDagProposerReasonTransactionPackThrottled) {
      LOG(log_tr_) << "Skip block proposer, transaction packing throttled by network state ..." << std::endl;
    }
  };
  auto finish_if_complete = [&](const rustaxa::DagProposerSessionStep& plan) -> std::optional<bool> {
    fail_on_invalid_report(plan);
    if (plan.status != kDagProposerSessionStatusComplete) {
      return std::nullopt;
    }
    log_terminal_skip(plan);
    return plan.return_value;
  };

  if (auto done = finish_if_complete(step)) {
    return *done;
  }
  if (step.action != kDagProposerSessionActionCollectExternalProposalFacts) {
    throw std::runtime_error("Rust DAG proposer session did not request external proposal facts");
  }

  const auto proposal_period = std::optional<PbftPeriod>{step.proposal_period};
  const auto final_chain_facts =
      final_chain_->dagProposerFinalChainFacts(proposal_period, node_dag_proposer_data->wallet.node_addr);
  auto sortition_params = dag_mgr_->sortitionParamsManager().rustSortitionParamsForRust(step.proposal_period);
  rustaxa::DagProposerExternalProposalFactsReport external_facts_report;
  external_facts_report.last_finalized_period = final_chain_facts.last_finalized_period;
  external_facts_report.authorization_facts = final_chain_facts.authorization_facts;
  external_facts_report.sortition_params = sortition_params;
  step = dag_mgr_->reportProposerExternalProposalFacts(proposer_session_id, std::move(external_facts_report));
  if (auto done = finish_if_complete(step)) {
    return *done;
  }
  if (step.action != kDagProposerSessionActionPackTransactions) {
    throw std::runtime_error("Rust DAG proposer session did not request transaction packing");
  }

  DagFrontier frontier(from_bridge_hash(step.frontier_pivot), from_bridge_dag_hashes(step.frontier_tips));
  LOG(log_dg_) << "Get frontier with pivot: " << frontier.pivot << " tips: " << frontier.tips;
  assert(!frontier.pivot.isZero());
  if (step.old_proposal) {
    LOG(log_wr_) << "Trying to propose old block " << step.proposal_level;
  }

  auto transaction_payloads =
      getShardedTrxs(step.transaction_request.proposal_period, step.transaction_request.weight_limit,
                     step.transaction_request.total_transaction_shards, step.transaction_request.node_transaction_shard,
                     step.transaction_request.shard_period_interval);
  rustaxa::DagProposerTransactionPackReport transaction_report;
  transaction_report.network_throttled = transaction_payloads.network_throttled;
  transaction_report.transaction_hashes.reserve(transaction_payloads.transaction_hashes.size());
  transaction_report.transaction_gas_estimations.reserve(transaction_payloads.gas_estimations.size());
  for (const auto& hash : transaction_payloads.transaction_hashes) {
    transaction_report.transaction_hashes.push_back(to_bridge_dag_hash(hash));
  }
  for (const auto estimation : transaction_payloads.gas_estimations) {
    transaction_report.transaction_gas_estimations.push_back(estimation);
  }
  step = dag_mgr_->reportProposerTransactions(proposer_session_id, std::move(transaction_report));
  if (auto done = finish_if_complete(step)) {
    return *done;
  }
  if (step.action != kDagProposerSessionActionStartVdf) {
    throw std::runtime_error("Rust DAG proposer session did not request VDF proof");
  }

  const auto vote_count = step.vote_count;
  const auto max_vote_count = step.max_vote_count;
  const auto vrf_input = to_bytes(step.vrf_input);
  dev::bytes vdf_msg = to_bytes(step.vdf_message);

  std::atomic_bool cancellation_token = false;
  std::promise<void> sync;
  auto rust_cancellation_token =
      rustaxa::make_cancellation_token_with_atomic(reinterpret_cast<const bool*>(&cancellation_token));
  std::optional<rustaxa::VdfSortitionProofResult> proof_result;
  executor_.post([params = to_legacy_sortition_params(sortition_params),
                  secret = node_dag_proposer_data->wallet.vrf_secret.asArray(), &vrf_input, &vdf_msg,
                  &rust_cancellation_token, &proof_result, &sync, vote_count, max_vote_count]() mutable {
    rust::Slice<const uint8_t> vrf_input_slice{vrf_input.data(), vrf_input.size()};
    rust::Slice<const uint8_t> vdf_input_slice{vdf_msg.data(), vdf_msg.size()};
    proof_result.emplace(rustaxa::prove_legacy_vdf_sortition(params, secret, vrf_input_slice, vdf_input_slice,
                                                             vote_count, max_vote_count, *rust_cancellation_token));
    sync.set_value();
  });

  std::future<void> result = sync.get_future();
  while (result.wait_for(std::chrono::milliseconds(step.vdf_poll_interval_ms)) != std::future_status::ready) {
    const auto wait_step = dag_mgr_->pollProposerVdfWait(proposer_session_id);
    fail_on_invalid_report(wait_step);
    if (wait_step.action == kDagProposerSessionActionCancelVdf) {
      cancellation_token = true;
      step = wait_step;
      break;
    }
  }

  if (cancellation_token) {
    result.wait();
    if (auto done = finish_if_complete(step)) {
      return *done;
    }
    throw std::runtime_error("Rust DAG proposer session did not complete after VDF cancellation");
  }

  if (!proof_result.has_value() || !proof_result->ok) {
    throw vdf_sortition::VdfSortition::InvalidVdfSortition(
        "Rust DAG proposer VDF proof failed. status " +
        std::to_string(proof_result.has_value() ? proof_result->status : 0) + ": " +
        (proof_result.has_value() ? std::string(proof_result->error) : std::string("missing proof result")));
  }

  auto vdf = vdf_sortition_from_proof(*proof_result);
  LOG(log_dg_) << node_dag_proposer_data->wallet.node_addr << " VDF difficulty " << vdf.getDifficulty();

  rustaxa::DagProposerVdfProofReport proof_report;
  proof_report.proof_ok = true;
  step = dag_mgr_->reportProposerVdfProof(proposer_session_id, std::move(proof_report));
  fail_on_invalid_report(step);

  if (step.action == kDagProposerSessionActionStaleProofSleep) {
    thisThreadSleepForMilliSeconds(step.stale_proof_sleep_ms);
    step = dag_mgr_->resumeProposerAfterStaleProofSleep(proposer_session_id);
    if (auto done = finish_if_complete(step)) {
      return *done;
    }
  }
  if (step.action != kDagProposerSessionActionBuildBlock) {
    throw std::runtime_error("Rust DAG proposer session did not request block construction");
  }

  auto selected_transaction_hashes = from_bridge_dag_hashes(step.selected_transaction_hashes);
  std::vector<uint64_t> selected_gas_estimations(step.transaction_gas_estimations.begin(),
                                                 step.transaction_gas_estimations.end());
  auto signed_block = createSignedDagBlockIntent(std::move(frontier), step.proposal_level, selected_transaction_hashes,
                                                 std::move(selected_gas_estimations), std::move(vdf),
                                                 node_dag_proposer_data->wallet.node_secret);
  rustaxa::DagProposerSigningReport signing_report;
  signing_report.signature_ready = true;
  step = dag_mgr_->reportProposerSigning(proposer_session_id, std::move(signing_report));
  fail_on_invalid_report(step);
  if (auto done = finish_if_complete(step)) {
    return *done;
  }
  if (step.action != kDagProposerSessionActionAddBlock) {
    throw std::runtime_error("Rust DAG proposer session did not request add-block execution");
  }

  const auto proposed_block_hash = from_bridge_hash(signed_block.block_hash);
  const auto proposed_transaction_count = selected_transaction_hashes.size();
  auto add_report = dag_mgr_->addDagBlockRlp(std::move(signed_block), selected_transaction_hashes,
                                             std::move(transaction_payloads.transaction_rlps), true);
  step = dag_mgr_->reportProposerAddBlock(proposer_session_id, std::move(add_report));
  if (step.record_proposed_block) {
    LOG(log_nf_) << node_dag_proposer_data->wallet.node_addr << " proposed new DAG block " << proposed_block_hash
                 << ", pivot " << from_bridge_hash(step.frontier_pivot) << ", txs num " << proposed_transaction_count;
    proposed_blocks_count_ += 1;
  } else {
    LOG(log_er_) << "Failed to add newly proposed dag block " << proposed_block_hash << ", proposed by "
                 << node_dag_proposer_data->wallet.node_addr << " into dag";
  }

  if (auto done = finish_if_complete(step)) {
    return *done;
  }
  throw std::runtime_error("Rust DAG proposer session did not complete after add-block report");
}

void DagBlockProposer::start() {
  if (bool b = true; !stopped_.compare_exchange_strong(b, !b)) {
    return;
  }
  LOG(log_nf_) << "DagBlockProposer started ...";

  proposed_blocks_count_ = 0;

  for (auto node_dag_proposer_data : nodes_dag_proposers_data_) {
    proposer_workers_.emplace_back(([this, node_dag_proposer_data]() {
      while (!stopped_) {
        auto syncing = false;
        auto packets_over_the_limit = false;
        if (auto net = network_.lock()) {
          syncing = net->pbft_syncing();
          packets_over_the_limit = net->packetQueueOverLimit();
        }
        rustaxa::DagProposerWorkerCommandInput command_input;
        command_input.pbft_syncing = syncing;
        command_input.packet_queue_over_limit = packets_over_the_limit;
        command_input.has_attempt_result = false;
        command_input.attempt_returned_proposed = false;
        auto command = rustaxa::dag_plan_proposer_worker_command(command_input);
        if (command.attempt_proposal) {
          command_input.has_attempt_result = true;
          command_input.attempt_returned_proposed = proposeDagBlock(node_dag_proposer_data);
          command = rustaxa::dag_plan_proposer_worker_command(command_input);
        }
        if (command.sleep_after_tick) {
          thisThreadSleepForMilliSeconds(command.sleep_ms);
        }
      }
    }));
  }
}

void DagBlockProposer::stop() {
  if (bool b = false; !stopped_.compare_exchange_strong(b, !b)) {
    return;
  }
  for (auto& proposer_worker : proposer_workers_) {
    if (proposer_worker.joinable()) {
      proposer_worker.join();
    }
  }

  LOG(log_nf_) << "DagBlockProposer stopped ...";
}

DagBlockProposer::ShardedProposalTransactions DagBlockProposer::getShardedTrxs(PbftPeriod proposal_period,
                                                                               uint64_t weight_limit,
                                                                               const uint16_t total_trx_shards,
                                                                               const uint16_t node_trx_shard,
                                                                               uint64_t shard_period_interval) const {
  auto syncing = false;
  if (auto net = network_.lock()) {
    syncing = net->pbft_syncing();
  }
  if (syncing) {
    ShardedProposalTransactions throttled;
    throttled.network_throttled = true;
    return throttled;
  }

  auto payloads = trx_mgr_->packShardedTransactionPayloads(proposal_period, weight_limit, total_trx_shards,
                                                           node_trx_shard, shard_period_interval);
  ShardedProposalTransactions transactions;
  transactions.transaction_hashes = std::move(payloads.transaction_hashes);
  transactions.transaction_rlps = std::move(payloads.transaction_rlps);
  transactions.gas_estimations = std::move(payloads.gas_estimations);
  return transactions;
}

vec_blk_t DagBlockProposer::selectDagBlockTips(const vec_blk_t& frontier_tips, uint64_t gas_limit) const {
  rustaxa::DagProposerStorageTipSelectionInput input;
  input.frontier_tips.reserve(frontier_tips.size());
  for (const auto& tip : frontier_tips) {
    input.frontier_tips.push_back(to_bridge_dag_hash(tip));
  }
  input.gas_limit = gas_limit;
  input.max_tips = kDagBlockMaxTips;

  const auto plan = dag_mgr_->planProposerTipSelection(std::move(input));
  return from_bridge_dag_hashes(plan.selected_tips);
}

rustaxa::DagProposerSignedBlockIntent DagBlockProposer::createSignedDagBlockIntent(
    DagFrontier&& frontier, level_t level, const vec_trx_t& trx_hashes, std::vector<uint64_t>&& estimations,
    VdfSortition&& vdf, const dev::Secret& node_secret) const {
  rustaxa::DagProposerStorageBlockConstructionInput plan_input;
  plan_input.pbft_gas_limit = kPbftGasLimit;
  plan_input.dag_gas_limit = kDagGasLimit;
  plan_input.max_tips = kDagBlockMaxTips;
  plan_input.frontier_tips.reserve(frontier.tips.size());
  for (const auto& t : frontier.tips) {
    plan_input.frontier_tips.push_back(to_bridge_dag_hash(t));
  }

  plan_input.transaction_gas_estimations.reserve(estimations.size());
  for (const auto estimation : estimations) {
    plan_input.transaction_gas_estimations.push_back(estimation);
  }

  const auto plan = dag_mgr_->planProposerBlockConstruction(std::move(plan_input));

  rustaxa::DagProposerBlockIntentNowInput intent_input;
  intent_input.pivot = to_bridge_hash(frontier.pivot);
  intent_input.level = level;
  intent_input.vdf_rlp = to_rust_vec(vdf.rlp());
  intent_input.selected_tips.reserve(plan.selected_tips.size());
  intent_input.transaction_hashes.reserve(trx_hashes.size());
  for (const auto& hash : plan.selected_tips) {
    intent_input.selected_tips.push_back(hash);
  }
  for (const auto& hash : trx_hashes) {
    intent_input.transaction_hashes.push_back(to_bridge_dag_hash(hash));
  }
  intent_input.block_gas_estimation = plan.block_gas_estimation;

  auto intent = rustaxa::dag_proposer_plan_block_intent_with_current_timestamp(std::move(intent_input));
  const auto signature = dev::sign(node_secret, from_bridge_hash(intent.signing_hash));
  rustaxa::DagProposerSignedBlockIntentInput signed_input;
  signed_input.intent = std::move(intent);
  signed_input.signature = to_rust_vec(signature.asBytes());
  return rustaxa::dag_proposer_finalize_signed_block_intent(std::move(signed_input));
}

void DagBlockProposer::setNetwork(std::weak_ptr<Network> network) { network_ = std::move(network); }

}  // namespace taraxa
