#pragma once

#include <memory>

#include "common/types.hpp"
#include "pbft/pbft_service.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {

struct FullNodeConfig;
class PbftVote;
class PillarVote;
class TransactionManager;
namespace final_chain {
class FinalChain;
}

/**
 * Rust-normalized double-vote evidence selected by PBFT vote admission.
 *
 * The two payload records are canonical signed PBFT votes for one shared
 * `(period, round, step)` slot. Keeping the slot once at this boundary avoids
 * re-expanding Rust-owned admission state into loose C++ scalar pairs.
 */
struct SlashingDoubleVoteEvidence {
  rustaxa::PbftVoteStorageRecord incoming_vote;
  rustaxa::PbftVoteStorageRecord conflicting_vote;
  PbftPeriod period = 0;
  PbftRound round = 0;
  PbftStep step = 0;
};

/**
 * Rust-mode SlashingManager facade.
 *
 * The public `SlashingManager` API is preserved while deterministic
 * double-voting proof planning is routed through the application-owned Rust
 * PBFT service. Submitter nonce and balance facts are read through the Rust
 * FinalChain runtime; C++ still owns transaction
 * construction, signing, and transaction-pool insertion. The facade has no
 * legacy implementation dependency.
 */
class SlashingManager {
 public:
  /**
   * Creates the Rust-mode slashing executor over the canonical PBFT service.
   *
   * `pbft_service` must be non-null and expose slashing capability; it owns the
   * planner configuration and duplicate-proof cache shared by every PBFT
   * facade. The remaining shared dependencies supply execution facts and
   * submit the transaction selected by Rust. Construction throws when the PBFT
   * service is missing or was created without slashing state.
   */
  SlashingManager(const FullNodeConfig& config, SharedPbftService pbft_service,
                  std::shared_ptr<final_chain::FinalChain> final_chain,
                  std::shared_ptr<TransactionManager> trx_manager);

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

  /**
   * Attempts to submit a double-voting proof from Rust-normalized evidence.
   *
   * Inputs:
   * - `evidence`: unweighted signed PBFT vote records plus the shared PBFT slot
   *   selected by the Rust admission runtime.
   *
   * Output and error behavior match the live-vote overload. This overload is
   * used by Rust-owned vote admission so slashing no longer needs a live C++
   * sidecar for the conflicting vote.
   */
  bool submitDoubleVotingProof(const SlashingDoubleVoteEvidence& evidence);

 private:
  rustaxa::DoubleVotingProofInput makeDoubleVotingProofInput(const SlashingDoubleVoteEvidence& evidence) const;
  bool submitDoubleVotingProofInput(rustaxa::DoubleVotingProofInput input);

  std::shared_ptr<final_chain::FinalChain> final_chain_;
  std::shared_ptr<TransactionManager> trx_manager_;
  SharedPbftService pbft_service_;
  const FullNodeConfig& kConfig;
};

}  // namespace taraxa
