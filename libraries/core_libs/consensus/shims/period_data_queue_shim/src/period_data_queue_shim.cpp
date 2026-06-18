#include "pbft/period_data_queue.hpp"

#include <stdexcept>
#include <utility>

#include "pbft/pbft_block.hpp"

namespace taraxa {
namespace {

std::runtime_error queueError(const std::string& message) { return std::runtime_error("PeriodDataQueue: " + message); }

std::runtime_error queueMismatch(const std::string& message, uint64_t expected, uint64_t actual) {
  return queueError(message + " expected entry " + std::to_string(expected) + ", got " + std::to_string(actual));
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
  last_block_cert_votes_.clear();
  next_entry_id_ = 1;
}

bool PeriodDataQueue::push(PeriodData&& period_data, const dev::p2p::NodeID& node_id, uint64_t max_pbft_size,
                           std::vector<std::shared_ptr<PbftVote>>&& cert_votes) {
  if (!period_data.pbft_blk) {
    throw queueError("cannot push period data without a PBFT block");
  }

  const auto period = period_data.pbft_blk->getPeriod();
  std::unique_lock lock(queue_access_);
  const auto entry_id = next_entry_id_;

  rustaxa::PeriodDataQueuePushOutcome outcome;
  try {
    outcome = rust_queue_->period_data_queue_push(entry_id, period, period_data.pbft_blk->getBlockHash().asArray(),
                                                  max_pbft_size, cert_votes.size());
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
  last_block_cert_votes_ = std::move(cert_votes);
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

const PeriodDataQueue::QueuedPayload& PeriodDataQueue::frontPayload(uint64_t expected_entry_id) const {
  if (queued_payloads_.empty()) {
    throw queueError("Rust pop selected next-entry cert votes while C++ payload queue is empty");
  }
  if (queued_payloads_.front().entry_id != expected_entry_id) {
    throw queueMismatch("next payload mismatch", expected_entry_id, queued_payloads_.front().entry_id);
  }
  return queued_payloads_.front();
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

std::tuple<PeriodData, std::vector<std::shared_ptr<PbftVote>>, dev::p2p::NodeID> PeriodDataQueue::pop() {
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
  if (!plan.use_last_block_cert_votes) {
    auto cert_votes = frontPayload(plan.next_entry_id).period_data.previous_block_cert_votes;
    return {std::move(payload.period_data), std::move(cert_votes), payload.node_id};
  }

  auto cert_votes = std::move(last_block_cert_votes_);
  last_block_cert_votes_.clear();
  return {std::move(payload.period_data), std::move(cert_votes), payload.node_id};
}

std::shared_ptr<PbftBlock> PeriodDataQueue::lastPbftBlock() const {
  std::shared_lock lock(queue_access_);
  const auto lookup = rust_queue_->period_data_queue_last_entry();
  if (!lookup.found) {
    return nullptr;
  }
  return backPayload(lookup.entry_id).period_data.pbft_blk;
}

std::optional<blk_hash_t> PeriodDataQueue::lastPbftBlockHash() const {
  std::shared_lock lock(queue_access_);
  const auto lookup = rust_queue_->period_data_queue_last_entry();
  if (!lookup.found) {
    return std::nullopt;
  }
  return blk_hash_t(lookup.block_hash.data(), blk_hash_t::ConstructFromPointer);
}

void PeriodDataQueue::cleanOldData(uint64_t period) {
  std::unique_lock lock(queue_access_);
  const auto removed_entries = rust_queue_->period_data_queue_clean_old_data(period);
  for (const auto& removed_entry : removed_entries) {
    (void)popFrontPayload(removed_entry.entry_id);
  }
}

}  // namespace taraxa
