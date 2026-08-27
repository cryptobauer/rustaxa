#pragma once

#ifdef RUSTAXA_ENABLE

#include <cstring>
#include <memory>
#include <stdexcept>
#include <utility>
#include <vector>

#include "consensus/consensus_application.hpp"
#include "consensus/consensus_host_ports.hpp"
#include "dag/dag_block_bundle_rlp.hpp"
#include "final_chain/final_chain.hpp"
#include "rustaxa-bridge/application_host_ffi.rs.h"
#include "vote/votes_bundle_rlp.hpp"

namespace taraxa::test {
namespace detail {

inline rust::Vec<uint8_t> finalizationBytes(const dev::bytes& bytes) {
  rust::Vec<uint8_t> result;
  result.reserve(bytes.size());
  for (const auto byte : bytes) result.push_back(byte);
  return result;
}

inline std::array<uint8_t, 32> finalizationHash(const h256& hash) {
  std::array<uint8_t, 32> result{};
  std::memcpy(result.data(), hash.data(), result.size());
  return result;
}

}  // namespace detail

/**
 * Finalizes one fixture period through the native application root.
 *
 * The helper is test-only compatibility ingress: it converts existing fixture
 * objects to canonical bytes, borrows the exact concrete-EVM leaf, and returns
 * only the typed report for an already-published FinalChain block. Invalid
 * canonical input, missing vote weights, or task rejection throws before the
 * fixture observes a successful publication.
 */
inline rustaxa::HostFinalChainFinalizeReport finalizeConsensusApplication(
    const SharedConsensusApplication& application, const std::shared_ptr<final_chain::FinalChain>& final_chain,
    PeriodData&& period_data, std::vector<h256> finalized_dag_hashes, uint32_t blocks_per_year,
    std::shared_ptr<DagBlock> anchor = nullptr) {
  rustaxa::HostFinalChainFinalizeTask task{};
  task.pbft_block_rlp = detail::finalizationBytes(period_data.pbft_blk->rlp(true));
  if (!period_data.previous_block_cert_votes.empty()) {
    task.previous_cert_vote_bundle_rlp =
        detail::finalizationBytes(encodePbftVotesBundleRlp(period_data.previous_block_cert_votes));
  }
  if (!period_data.dag_blocks.empty()) {
    task.dag_block_bundle_rlp = detail::finalizationBytes(encodeDAGBlocksBundleRlp(period_data.dag_blocks));
  }
  task.transaction_rlps.reserve(period_data.transactions.size());
  for (const auto& transaction : period_data.transactions) {
    rustaxa::CanonicalBytes bytes{};
    bytes.data = detail::finalizationBytes(transaction->rlp());
    task.transaction_rlps.push_back(std::move(bytes));
  }
  task.previous_cert_votes.reserve(period_data.previous_block_cert_votes.size());
  for (const auto& vote : period_data.previous_block_cert_votes) {
    const auto weight = vote->getWeight();
    if (!weight) throw std::runtime_error("FinalChain fixture cert vote is missing validator weight");
    rustaxa::HostRewardCertVote fact{};
    fact.rlp = detail::finalizationBytes(vote->rlp(true));
    fact.weight = *weight;
    task.previous_cert_votes.push_back(std::move(fact));
  }
  task.finalized_dag_hashes.reserve(finalized_dag_hashes.size());
  for (const auto& hash : finalized_dag_hashes) {
    task.finalized_dag_hashes.push_back(rustaxa::DagHash{detail::finalizationHash(hash)});
  }
  task.blocks_per_year = blocks_per_year;
  if (anchor) task.anchor_block_rlp = detail::finalizationBytes(anchor->rlp(true));

  ExternalEvmPort external_evm(final_chain);
  auto report = application->finalize(external_evm, std::move(task));
  if (!report.error_code.empty()) {
    throw std::runtime_error("FinalChain application fixture task failed: " + std::string(report.error_code));
  }
  return report;
}

}  // namespace taraxa::test

#endif
