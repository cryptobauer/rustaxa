#pragma once

#include <memory>

#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

struct FullNodeConfig;
class GasPricer;
class PbftVote;
class TransactionManager;
namespace final_chain {
class FinalChain;
}

/**
 * Rust-mode SlashingManager facade.
 *
 * The public `SlashingManager` API is preserved while deterministic
 * double-voting proof planning is routed through Rust. C++ still owns live
 * FinalChain account reads, gas-price lookup, transaction construction,
 * signing, and transaction-pool insertion. No production path delegates to
 * `SlashingManagerOld`.
 */
class SlashingManager {
 public:
  SlashingManager(const FullNodeConfig& config, std::shared_ptr<final_chain::FinalChain> final_chain,
                  std::shared_ptr<TransactionManager> trx_manager, std::shared_ptr<GasPricer> gas_pricer);

  SlashingManager(const SlashingManager&) = delete;
  SlashingManager(SlashingManager&&) = delete;
  SlashingManager& operator=(const SlashingManager&) = delete;
  SlashingManager& operator=(SlashingManager&&) = delete;

  /**
   * Attempts to submit a double-voting proof.
   *
   * Inputs:
   * - `vote_a`, `vote_b`: PBFT vote objects with matching PBFT slot metadata
   *   and same slot evidence to construct one slashing transaction payload.
   *
   * Output:
   * - true when a transaction was accepted by the transaction manager.
   * - false when report is disabled, duplicate proof was already planned, or
   *   wallet/validation constraints prevent submission.
   *
   * Error/edge behavior:
   * - expected rejection paths return false through explicit Rust planner
   *   statuses
   * - bridge or invariant failures are surfaced as exceptions
   */
  bool submitDoubleVotingProof(const std::shared_ptr<PbftVote>& vote_a, const std::shared_ptr<PbftVote>& vote_b);

 private:
  std::shared_ptr<final_chain::FinalChain> final_chain_;
  std::shared_ptr<TransactionManager> trx_manager_;
  std::shared_ptr<GasPricer> gas_pricer_;
  ::rust::Box<rustaxa::BridgeSlashingProofPlanner> planner_;
  const FullNodeConfig& kConfig;
};

}  // namespace taraxa
