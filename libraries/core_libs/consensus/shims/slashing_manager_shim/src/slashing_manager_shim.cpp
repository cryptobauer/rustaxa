#include "slashing_manager/slashing_manager.hpp"

#include <algorithm>
#include <cstring>
#include <stdexcept>
#include <utility>

#include "config/config.hpp"
#include "final_chain/final_chain.hpp"
#include "transaction/gas_pricer.hpp"
#include "transaction/transaction_manager.hpp"
#include "vote/pbft_vote.hpp"

namespace taraxa {
namespace {

std::array<uint8_t, 32> to_bridge_hash(const dev::h256& hash) {
  std::array<uint8_t, 32> out{};
  std::memcpy(out.data(), hash.data(), out.size());
  return out;
}

std::array<uint8_t, 32> to_bridge_u256(const u256& value) {
  std::array<uint8_t, 32> out{};
  const auto bytes = dev::toBigEndian(value);
  if (bytes.size() > out.size()) {
    throw std::overflow_error("u256 value cannot be represented in 32 bridge bytes");
  }
  std::copy(bytes.begin(), bytes.end(), out.begin() + (out.size() - bytes.size()));
  return out;
}

std::array<uint8_t, 32> to_bridge_account_balance(const rust::Vec<uint8_t>& value) {
  std::array<uint8_t, 32> out{};
  if (value.size() > out.size()) {
    throw std::overflow_error("Rust FinalChain account balance cannot be represented in 32 bridge bytes");
  }
  std::copy(value.begin(), value.end(), out.begin() + (out.size() - value.size()));
  return out;
}

u256 from_bridge_u256(const std::array<uint8_t, 32>& value) {
  return dev::fromBigEndian<u256>(dev::bytes(value.begin(), value.end()));
}

rust::Vec<uint8_t> to_bridge_bytes(const bytes& input) {
  rust::Vec<uint8_t> out;
  out.reserve(input.size());
  for (const auto byte : input) {
    out.push_back(byte);
  }
  return out;
}

rust::Slice<const uint8_t> to_bridge_byte_slice(const rust::Vec<uint8_t>& bytes) {
  return rust::Slice<const uint8_t>(bytes.data(), bytes.size());
}

addr_t from_bridge_address(const std::array<uint8_t, 20>& address) {
  return addr_t(address.data(), addr_t::ConstructFromPointer);
}

rustaxa::PbftVoteStorageRecord make_slashing_vote_payload(const std::shared_ptr<PbftVote>& vote) {
  if (!vote) {
    throw std::runtime_error("SlashingManager cannot build a payload for a null PBFT vote");
  }

  auto canonical_vote_rlp = to_bridge_bytes(vote->rlp(true, false));
  auto record = rustaxa::pbft_vote_slashing_payload_from_canonical_vote(to_bridge_byte_slice(canonical_vote_rlp));
  if (record.hash != to_bridge_hash(vote->getHash())) {
    throw std::runtime_error("Rust PBFT slashing payload hash mismatches live vote hash");
  }
  return record;
}

rustaxa::PbftVoteStorageRecord clone_vote_record(const rustaxa::PbftVoteStorageRecord& record) {
  rustaxa::PbftVoteStorageRecord out;
  out.hash = record.hash;
  out.vote_rlp.reserve(record.vote_rlp.size());
  for (const auto byte : record.vote_rlp) {
    out.vote_rlp.push_back(byte);
  }
  return out;
}

rust::Vec<rustaxa::SlashingSubmitterFact> submitter_facts(const FullNodeConfig& config,
                                                          const std::shared_ptr<final_chain::FinalChain>& final_chain) {
  rust::Vec<rustaxa::SlashingSubmitterFact> facts;
  facts.reserve(config.wallets.size());
  for (size_t index = 0; index < config.wallets.size(); ++index) {
    const auto& wallet = config.wallets[index];
    rustaxa::SlashingSubmitterFact fact{};
    fact.wallet_index = index;
    const auto account = final_chain->rustFinalChainForRust().get_account(wallet.node_addr.asArray());
    if (account.found) {
      fact.nonce = to_bridge_u256(u256(account.nonce));
      fact.balance = to_bridge_account_balance(account.balance);
    }
    facts.push_back(std::move(fact));
  }
  return facts;
}

}  // namespace

SlashingManager::SlashingManager(const FullNodeConfig& config, std::shared_ptr<final_chain::FinalChain> final_chain,
                                 std::shared_ptr<TransactionManager> trx_manager,
                                 std::shared_ptr<GasPricer> gas_pricer)
    : final_chain_(std::move(final_chain)),
      trx_manager_(std::move(trx_manager)),
      gas_pricer_(std::move(gas_pricer)),
      planner_(rustaxa::create_slashing_proof_planner(config.report_malicious_behaviour)),
      kConfig(config) {}

bool SlashingManager::submitDoubleVotingProof(const std::shared_ptr<PbftVote>& vote_a,
                                              const std::shared_ptr<PbftVote>& vote_b) {
  if (!vote_a || !vote_b) {
    return false;
  }
  if (!final_chain_ || !trx_manager_ || !gas_pricer_) {
    throw std::logic_error("SlashingManager requires FinalChain, TransactionManager, and GasPricer");
  }

  auto vote_a_payload = make_slashing_vote_payload(vote_a);
  auto vote_b_payload = make_slashing_vote_payload(vote_b);

  rustaxa::DoubleVotingProofInput input;
  input.vote_a_hash = vote_a_payload.hash;
  input.vote_b_hash = vote_b_payload.hash;
  input.vote_a_period = vote_a->getPeriod();
  input.vote_b_period = vote_b->getPeriod();
  input.vote_a_round = vote_a->getRound();
  input.vote_b_round = vote_b->getRound();
  input.vote_a_step = vote_a->getStep();
  input.vote_b_step = vote_b->getStep();
  input.vote_a_rlp = std::move(vote_a_payload.vote_rlp);
  input.vote_b_rlp = std::move(vote_b_payload.vote_rlp);
  input.submitters = submitter_facts(kConfig, final_chain_);

  return submitDoubleVotingProofInput(std::move(input));
}

bool SlashingManager::submitDoubleVotingProof(const rustaxa::PbftVoteStorageRecord& vote_a,
                                              const rustaxa::PbftVoteStorageRecord& vote_b, PbftPeriod period,
                                              PbftRound round, PbftStep step) {
  if (vote_a.vote_rlp.empty() || vote_b.vote_rlp.empty()) {
    return false;
  }
  if (!final_chain_ || !trx_manager_ || !gas_pricer_) {
    throw std::logic_error("SlashingManager requires FinalChain, TransactionManager, and GasPricer");
  }

  auto vote_a_payload = clone_vote_record(vote_a);
  auto vote_b_payload = clone_vote_record(vote_b);

  rustaxa::DoubleVotingProofInput input;
  input.vote_a_hash = vote_a_payload.hash;
  input.vote_b_hash = vote_b_payload.hash;
  input.vote_a_period = period;
  input.vote_b_period = period;
  input.vote_a_round = round;
  input.vote_b_round = round;
  input.vote_a_step = step;
  input.vote_b_step = step;
  input.vote_a_rlp = std::move(vote_a_payload.vote_rlp);
  input.vote_b_rlp = std::move(vote_b_payload.vote_rlp);
  input.submitters = submitter_facts(kConfig, final_chain_);

  return submitDoubleVotingProofInput(std::move(input));
}

bool SlashingManager::submitDoubleVotingProofInput(rustaxa::DoubleVotingProofInput input) {
  const auto plan = planner_->slashing_plan_double_voting_proof(std::move(input));
  if (!plan.should_submit) {
    return false;
  }
  if (plan.wallet_index >= kConfig.wallets.size()) {
    throw std::runtime_error("Rust slashing planner returned an invalid wallet index");
  }

  const auto& wallet = kConfig.wallets[plan.wallet_index];
  auto call_data = bytes(plan.call_data.begin(), plan.call_data.end());
  auto trx = std::make_shared<Transaction>(from_bridge_u256(plan.nonce), from_bridge_u256(plan.value),
                                           gas_pricer_->bid(), plan.gas_limit, std::move(call_data), wallet.node_secret,
                                           from_bridge_address(plan.contract_address), kConfig.genesis.chain_id);

  rustaxa::DoubleVotingProofSubmissionReport report;
  report.proof_hash = plan.proof_hash;
  report.transaction_inserted = trx_manager_->insertTransaction(trx).first;
  const auto submission_plan = planner_->slashing_report_double_voting_proof_submission(std::move(report));
  return submission_plan.submitted;
}

}  // namespace taraxa
