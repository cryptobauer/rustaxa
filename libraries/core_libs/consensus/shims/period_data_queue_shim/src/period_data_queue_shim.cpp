#include <stdexcept>
#include <utility>

#include "pbft/pbft_block.hpp"
#include "pbft/period_data_queue.hpp"
#include "vote/pbft_vote.hpp"
#include "vote/pillar_vote.hpp"

namespace taraxa {
namespace {

constexpr uint8_t kLegacyTransactionSourceRegular = 0;

std::runtime_error queueError(const std::string& message) { return std::runtime_error("PeriodDataQueue: " + message); }

std::runtime_error queueMismatch(const std::string& message, uint64_t expected, uint64_t actual) {
  return queueError(message + " expected entry " + std::to_string(expected) + ", got " + std::to_string(actual));
}

rust::Vec<uint8_t> toBridgeBytes(const bytes& input) {
  rust::Vec<uint8_t> out;
  out.reserve(input.size());
  for (const auto byte : input) {
    out.push_back(byte);
  }
  return out;
}

rust::Vec<rustaxa::PbftSyncTransactionHash> toBridgeTransactionHashes(const std::vector<trx_hash_t>& hashes) {
  rust::Vec<rustaxa::PbftSyncTransactionHash> out;
  out.reserve(hashes.size());
  for (const auto& hash : hashes) {
    out.push_back(rustaxa::PbftSyncTransactionHash{hash.asArray()});
  }
  return out;
}

std::vector<trx_hash_t> fromBridgeTransactionHashes(const rust::Vec<rustaxa::PbftSyncTransactionHash>& hashes) {
  std::vector<trx_hash_t> out;
  out.reserve(hashes.size());
  for (const auto& hash : hashes) {
    out.emplace_back(hash.hash.data(), trx_hash_t::ConstructFromPointer);
  }
  return out;
}

std::vector<trx_hash_t> dagTransactionHashes(const PeriodData& period_data) {
  std::vector<trx_hash_t> hashes;
  for (const auto& dag_block : period_data.dag_blocks) {
    for (const auto& trx_hash : dag_block->getTrxs()) {
      hashes.emplace_back(trx_hash);
    }
  }
  return hashes;
}

std::vector<trx_hash_t> periodDataTransactionHashes(const PeriodData& period_data) {
  std::vector<trx_hash_t> hashes;
  hashes.reserve(period_data.transactions.size());
  for (const auto& transaction : period_data.transactions) {
    hashes.emplace_back(transaction->getHash());
  }
  return hashes;
}

rust::Vec<rustaxa::PeriodDataQueueTransactionPayload> periodDataTransactionRlps(const PeriodData& period_data) {
  rust::Vec<rustaxa::PeriodDataQueueTransactionPayload> payloads;
  payloads.reserve(period_data.transactions.size());
  for (const auto& transaction : period_data.transactions) {
    if (!transaction) {
      throw queueError("cannot push period data with a null transaction");
    }
    rustaxa::PeriodDataQueueTransactionPayload payload;
    payload.transaction_rlp = toBridgeBytes(transaction->rlp());
    payloads.push_back(std::move(payload));
  }
  return payloads;
}

rust::Vec<rustaxa::PeriodDataQueuePbftVotePayload> pbftVoteRlps(
    const std::vector<std::shared_ptr<PbftVote>>& votes) {
  rust::Vec<rustaxa::PeriodDataQueuePbftVotePayload> payloads;
  payloads.reserve(votes.size());
  for (const auto& vote : votes) {
    if (!vote) {
      throw queueError("cannot push period data with a null PBFT cert vote");
    }
    rustaxa::PeriodDataQueuePbftVotePayload payload;
    payload.vote_rlp = toBridgeBytes(vote->rlp(true, vote->getWeight().has_value()));
    payloads.push_back(std::move(payload));
  }
  return payloads;
}

std::vector<vote_hash_t> rewardVoteHashes(const PeriodData& period_data) {
  return period_data.pbft_blk->getRewardVotes();
}

rust::Vec<rustaxa::PeriodDataQueuePillarVotePayload> pillarVoteRlps(const PeriodData& period_data) {
  rust::Vec<rustaxa::PeriodDataQueuePillarVotePayload> payloads;
  if (!period_data.pillar_votes_.has_value()) {
    return payloads;
  }

  payloads.reserve(period_data.pillar_votes_->size());
  for (const auto& vote : *period_data.pillar_votes_) {
    if (!vote) {
      throw queueError("cannot push period data with a null pillar vote");
    }
    rustaxa::PeriodDataQueuePillarVotePayload payload;
    payload.vote_rlp = toBridgeBytes(vote->rlp());
    payloads.push_back(std::move(payload));
  }
  return payloads;
}

std::vector<bytes> fromBridgePillarVoteRlps(const rust::Vec<rustaxa::PeriodDataQueuePillarVotePayload>& payloads) {
  std::vector<bytes> out;
  out.reserve(payloads.size());
  for (const auto& payload : payloads) {
    out.emplace_back(payload.vote_rlp.begin(), payload.vote_rlp.end());
  }
  return out;
}

std::vector<bytes> fromBridgeTransactionRlps(const rust::Vec<rustaxa::PeriodDataQueueTransactionPayload>& payloads) {
  std::vector<bytes> out;
  out.reserve(payloads.size());
  for (const auto& payload : payloads) {
    out.emplace_back(payload.transaction_rlp.begin(), payload.transaction_rlp.end());
  }
  return out;
}

std::vector<bytes> fromBridgePbftVoteRlps(const rust::Vec<rustaxa::PeriodDataQueuePbftVotePayload>& payloads) {
  std::vector<bytes> out;
  out.reserve(payloads.size());
  for (const auto& payload : payloads) {
    out.emplace_back(payload.vote_rlp.begin(), payload.vote_rlp.end());
  }
  return out;
}

std::vector<std::shared_ptr<PbftVote>> materializePbftVotesFromQueuedRlps(const std::vector<bytes>& vote_rlps) {
  std::vector<std::shared_ptr<PbftVote>> votes;
  votes.reserve(vote_rlps.size());
  for (const auto& vote_rlp : vote_rlps) {
    if (vote_rlp.empty()) {
      throw queueError("cannot materialize an empty PBFT cert-vote payload");
    }
    votes.emplace_back(std::make_shared<PbftVote>(vote_rlp));
  }
  return votes;
}

rust::Vec<rustaxa::PeriodDataQueueTransactionIdentity> periodDataTransactionIdentities(const PeriodData& period_data) {
  rust::Vec<rustaxa::PeriodDataQueueTransactionIdentity> identities;
  identities.reserve(period_data.transactions.size());

  uint64_t input_index = 0;
  for (const auto& transaction : period_data.transactions) {
    if (!transaction) {
      throw queueError("cannot push period data with a null transaction");
    }
    rustaxa::LegacyTransactionInspection inspection;
    try {
      inspection = rustaxa::inspect_legacy_transaction_rlp(toBridgeBytes(transaction->rlp()),
                                                           kLegacyTransactionSourceRegular);
    } catch (const std::exception& e) {
      throw queueError(std::string("transaction identity inspection failed: ") + e.what());
    }
    if (trx_hash_t(inspection.hash.data(), trx_hash_t::ConstructFromPointer) != transaction->getHash()) {
      throw queueError("transaction identity inspection returned mismatched hash");
    }
    if (!inspection.sender_found) {
      throw queueError("transaction identity inspection returned no recovered sender");
    }

    rustaxa::PeriodDataQueueTransactionIdentity identity;
    identity.input_index = input_index++;
    identity.hash = inspection.hash;
    identity.transaction_nonce = inspection.nonce;
    identity.sender = inspection.sender;
    identities.push_back(identity);
  }
  return identities;
}

rust::Vec<rustaxa::TransactionManagerVerifyNotFinalizedRuntimeFact> toVerifyNotFinalizedFacts(
    const rust::Vec<rustaxa::PeriodDataQueueTransactionIdentity>& identities) {
  rust::Vec<rustaxa::TransactionManagerVerifyNotFinalizedRuntimeFact> facts;
  facts.reserve(identities.size());
  for (const auto& identity : identities) {
    rustaxa::TransactionManagerVerifyNotFinalizedRuntimeFact fact;
    fact.input_index = identity.input_index;
    fact.hash = identity.hash;
    fact.transaction_nonce = identity.transaction_nonce;
    fact.sender = identity.sender;
    facts.push_back(fact);
  }
  return facts;
}

bool previousCertFirstVoteHasWeight(const PeriodData& period_data) {
  return !period_data.previous_block_cert_votes.empty() &&
         period_data.previous_block_cert_votes.front()->getWeight().has_value();
}

bool extraDataPillarBlockHashPresent(const std::shared_ptr<PbftBlock>& pbft_block) {
  const auto extra_data = pbft_block->getExtraData();
  return extra_data.has_value() && extra_data->getPillarBlockHash().has_value();
}

}  // namespace

PeriodDataQueue::PeriodDataQueue() : rust_queue_(rustaxa::create_period_data_queue()) {}

PeriodDataQueue::~PeriodDataQueue() = default;

uint64_t PeriodDataQueue::getPeriod() const {
  std::shared_lock lock(queue_access_);
  return rust_queue_->period_data_queue_period();
}

uint64_t PeriodDataQueue::syncingPeriod(uint64_t pbft_chain_size) const {
  std::shared_lock lock(queue_access_);
  return rust_queue_->period_data_queue_syncing_period(pbft_chain_size);
}

blk_hash_t PeriodDataQueue::lastBlockHashOrChain(uint64_t current_period, const blk_hash_t& chain_last_hash) const {
  std::shared_lock lock(queue_access_);
  const auto hash = rust_queue_->period_data_queue_last_block_hash_or_chain(current_period, chain_last_hash.asArray());
  return blk_hash_t(hash.data(), blk_hash_t::ConstructFromPointer);
}

size_t PeriodDataQueue::size() const {
  std::shared_lock lock(queue_access_);
  return rust_queue_->period_data_queue_size();
}

bool PeriodDataQueue::empty() const {
  std::shared_lock lock(queue_access_);
  return rust_queue_->period_data_queue_empty();
}

void PeriodDataQueue::clear() {
  std::unique_lock lock(queue_access_);
  rust_queue_->period_data_queue_clear();
  queued_payloads_.clear();
  next_entry_id_ = 1;
}

bool PeriodDataQueue::push(PeriodData&& period_data, const dev::p2p::NodeID& node_id, uint64_t max_pbft_size,
                           std::vector<std::shared_ptr<PbftVote>>&& cert_votes) {
  if (!period_data.pbft_blk) {
    throw queueError("cannot push period data without a PBFT block");
  }

  const auto period = period_data.pbft_blk->getPeriod();
  const auto reward_vote_hashes = rewardVoteHashes(period_data);
  auto pillar_vote_rlps = pillarVoteRlps(period_data);
  auto transaction_rlps = periodDataTransactionRlps(period_data);
  auto previous_cert_vote_rlps = pbftVoteRlps(period_data.previous_block_cert_votes);
  auto current_block_cert_vote_rlps = pbftVoteRlps(cert_votes);
  const auto dag_transaction_hashes = dagTransactionHashes(period_data);
  const auto period_data_transaction_hashes = periodDataTransactionHashes(period_data);
  auto period_data_transaction_identities = periodDataTransactionIdentities(period_data);
  const auto previous_cert_votes_present = !period_data.previous_block_cert_votes.empty();
  const auto previous_cert_first_vote_has_weight = previousCertFirstVoteHasWeight(period_data);
  const auto pillar_votes_present = period_data.pillar_votes_.has_value();
  const auto extra_data_present = period_data.pbft_blk->getExtraData().has_value();
  const auto extra_data_pillar_block_hash_present = extraDataPillarBlockHashPresent(period_data.pbft_blk);
  std::unique_lock lock(queue_access_);
  const auto entry_id = next_entry_id_;

  rustaxa::PeriodDataQueuePushOutcome outcome;
  try {
    outcome = rust_queue_->period_data_queue_push(
        entry_id, period, period_data.pbft_blk->getBlockHash().asArray(),
        period_data.pbft_blk->getPrevBlockHash().asArray(), period_data.pbft_blk->getPivotDagBlockHash().asArray(),
        period_data.pbft_blk->getFinalChainHash().asArray(), toBridgeTransactionHashes(reward_vote_hashes),
        std::move(pillar_vote_rlps), std::move(transaction_rlps), std::move(previous_cert_vote_rlps),
        toBridgeTransactionHashes(dag_transaction_hashes), toBridgeTransactionHashes(period_data_transaction_hashes),
        std::move(period_data_transaction_identities), previous_cert_votes_present,
        previous_cert_first_vote_has_weight, pillar_votes_present, extra_data_present,
        extra_data_pillar_block_hash_present, max_pbft_size, std::move(current_block_cert_vote_rlps));
  } catch (const std::exception& e) {
    throw queueError(e.what());
  } catch (...) {
    throw queueError("Rust push failed");
  }

  if (!outcome.accepted) {
    return false;
  }

  if (outcome.clear_existing) {
    queued_payloads_.clear();
  }

  queued_payloads_.push_back(QueuedPayload{entry_id, std::move(period_data), node_id});
  ++next_entry_id_;
  return true;
}

PeriodDataQueue::QueuedPayload PeriodDataQueue::popFrontPayload(uint64_t expected_entry_id) {
  if (queued_payloads_.empty()) {
    throw queueError("Rust pop selected an entry while C++ payload queue is empty");
  }
  if (queued_payloads_.front().entry_id != expected_entry_id) {
    throw queueMismatch("front payload mismatch", expected_entry_id, queued_payloads_.front().entry_id);
  }

  auto payload = std::move(queued_payloads_.front());
  queued_payloads_.pop_front();
  return payload;
}

const PeriodDataQueue::QueuedPayload& PeriodDataQueue::backPayload(uint64_t expected_entry_id) const {
  if (queued_payloads_.empty()) {
    throw queueError("Rust last-entry lookup succeeded while C++ payload queue is empty");
  }
  if (queued_payloads_.back().entry_id != expected_entry_id) {
    throw queueMismatch("last payload mismatch", expected_entry_id, queued_payloads_.back().entry_id);
  }
  return queued_payloads_.back();
}

PeriodDataQueue::PoppedPeriodData PeriodDataQueue::popWithMetadata() {
  std::unique_lock lock(queue_access_);

  rustaxa::PeriodDataQueuePopPlan plan;
  try {
    plan = rust_queue_->period_data_queue_pop();
  } catch (const std::exception& e) {
    throw queueError(e.what());
  } catch (...) {
    throw queueError("Rust pop failed");
  }

  auto payload = popFrontPayload(plan.entry_id);
  auto result = PoppedPeriodData{std::move(payload.period_data),
                                 {},
                                 payload.node_id,
                                 plan.entry_period,
                                 blk_hash_t(plan.block_hash.data(), blk_hash_t::ConstructFromPointer),
                                 blk_hash_t(plan.prev_block_hash.data(), blk_hash_t::ConstructFromPointer),
                                 blk_hash_t(plan.pivot_hash.data(), blk_hash_t::ConstructFromPointer),
                                 blk_hash_t(plan.final_chain_hash.data(), blk_hash_t::ConstructFromPointer),
                                 fromBridgeTransactionHashes(plan.reward_vote_hashes),
                                 fromBridgePillarVoteRlps(plan.pillar_vote_rlps),
                                 fromBridgeTransactionRlps(plan.transaction_rlps),
                                 fromBridgePbftVoteRlps(plan.cert_vote_rlps),
                                 fromBridgeTransactionHashes(plan.dag_transaction_hashes),
                                 fromBridgeTransactionHashes(plan.period_data_transaction_hashes),
                                 toVerifyNotFinalizedFacts(plan.period_data_transaction_identities),
                                 plan.previous_cert_votes_present,
                                 plan.previous_cert_first_vote_has_weight,
                                 plan.pillar_votes_present,
                                 plan.extra_data_present,
                                 plan.extra_data_pillar_block_hash_present};
  result.cert_votes = materializePbftVotesFromQueuedRlps(result.cert_vote_rlps);
  return result;
}

void PeriodDataQueue::cleanOldData(uint64_t period) {
  std::unique_lock lock(queue_access_);
  const auto removed_entries = rust_queue_->period_data_queue_clean_old_data(period);
  for (const auto& removed_entry : removed_entries) {
    (void)popFrontPayload(removed_entry.entry_id);
  }
}

}  // namespace taraxa
