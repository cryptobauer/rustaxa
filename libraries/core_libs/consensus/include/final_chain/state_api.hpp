#pragma once

#include <taraxa-evm/taraxa-evm.h>

#include <functional>

#include "final_chain/state_api_data.hpp"
#include "rewards/block_stats.hpp"

namespace taraxa::state_api {

struct Config;
struct Opts;
struct OptsDB;

/** @addtogroup FinalChain
 * @{
 */

class StateAPI {
  std::function<h256(EthBlockNumber)> get_blk_hash_;
  taraxa_evm_GetBlockHash get_blk_hash_c_;
  taraxa_evm_state_API_ptr this_c_;
  dev::RLPStream rlp_enc_execution_result_;
  TransactionsExecutionResult result_buf_execution_result_;
  h256 post_transaction_state_root_;
  dev::RLPStream rlp_enc_rewards_distribution_;
  RewardsDistributionResult result_buf_rewards_distribution_;
  std::string db_path_;

 public:
  StateAPI(std::function<h256(EthBlockNumber)> get_blk_hash, const Config& state_config, const Opts& opts,
           const OptsDB& opts_db);
  ~StateAPI();
  StateAPI(const StateAPI&) = default;
  StateAPI(StateAPI&&) = default;
  StateAPI& operator=(const StateAPI&) = default;
  StateAPI& operator=(StateAPI&&) = default;

  void update_state_config(const Config& new_config);

  std::optional<Account> get_account(EthBlockNumber blk_num, const addr_t& addr) const;
  h256 get_account_storage(EthBlockNumber blk_num, const addr_t& addr, const u256& key) const;
  bytes get_code_by_address(EthBlockNumber blk_num, const addr_t& addr) const;
  ExecutionResult dry_run_transaction(EthBlockNumber blk_num, const EVMBlock& blk, const EVMTransaction& trx) const;
  bytes trace(EthBlockNumber blk_num, const EVMBlock& blk, const std::vector<EVMTransaction>& state_trxs,
              const std::vector<EVMTransaction>& trxs, std::optional<Tracing> params = {}) const;
  StateDescriptor get_last_committed_state_descriptor() const;

#ifdef RUSTAXA_ENABLE
  /**
   * Activates the versioned concrete-root policy for `chain_identity` and returns its canonical provenance RLP.
   *
   * A fresh database persists a stable database identity. Reopening an initialized database requires the same chain
   * identity and policy version; markerless, synthetic, or differently paired state is rejected by StateAPI.
   */
  bytes activate_concrete_root_policy(const h256& chain_identity);
  /** Returns the canonical provenance RLP paired with the last committed StateAPI descriptor. */
  bytes get_concrete_state_provenance() const;
  /** Returns the exact durable staged-execution marker RLP, or no value when the physical state is clean. */
  std::optional<bytes> get_pending_concrete_execution() const;
  /** Durably stages Rust's canonical marker RLP before any concrete mutation; conflicting retries fail closed. */
  void stage_concrete_execution(const bytes& marker_rlp);
  /** Returns StateAPI's versioned canonical projection RLP for the currently prepared concrete state. */
  bytes get_concrete_state_projection() const;
  /**
   * Commits the prepared state only when its projection hash matches, atomically pairing Rust's exact provenance RLP.
   */
  void concrete_commit(const h256& expected_projection_hash, const bytes& committed_provenance_rlp);
  /**
   * Discards the exact staged marker, clears pending physical writes, and reopens at the committed descriptor.
   *
   * A marker mismatch or reopen failure is an error and leaves callers unable to treat the executor as clean.
   */
  void discard_concrete_execution(const bytes& exact_marker_rlp);
#endif

  const TransactionsExecutionResult& execute_transactions(const EVMBlock& block,
                                                          const std::vector<EVMTransaction>& transactions);
  /** Returns the exact trie root after the most recent transaction phase and before rewards. */
  h256 post_transaction_state_root() const { return post_transaction_state_root_; }
  const RewardsDistributionResult& distribute_rewards(const std::vector<rewards::BlockStats>& rewards_stats);
  void transition_state_commit();

  void create_snapshot(PbftPeriod period);
  void prune(const std::vector<dev::h256>& state_root_to_keep, EthBlockNumber blk_num);

  // DPOS
  uint64_t dpos_eligible_total_vote_count(EthBlockNumber blk_num) const;
  uint64_t dpos_eligible_vote_count(EthBlockNumber blk_num, const addr_t& addr) const;
  bool dpos_is_eligible(EthBlockNumber blk_num, const addr_t& addr) const;
  u256 get_staking_balance(EthBlockNumber blk_num, const addr_t& addr) const;
  vrf_wrapper::vrf_pk_t dpos_get_vrf_key(EthBlockNumber blk_num, const addr_t& addr) const;
  std::vector<ValidatorStake> dpos_validators_total_stakes(EthBlockNumber blk_num) const;
  std::vector<ValidatorVoteCount> dpos_validators_eligible_vote_counts(EthBlockNumber blk_num) const;
  uint64_t dpos_yield(EthBlockNumber blk_num) const;
  u256 dpos_total_supply(EthBlockNumber blk_num) const;
  u256 dpos_total_amount_delegated(EthBlockNumber blk_num) const;
};
/** @} */

}  // namespace taraxa::state_api

namespace taraxa {
using state_api::StateAPI;
}
