#pragma once

#include <deque>
#include <optional>
#include <shared_mutex>
#include <string_view>
#include <thread>

#include "common/types.hpp"
#include "config/config.hpp"
#include "final_chain/final_chain.hpp"
#include "logger/logger.hpp"
#include "network/network.hpp"
#include "pbft/period_data.hpp"
#include "pbft/proposed_blocks.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "vote/pillar_vote.hpp"

namespace taraxa {

/** @addtogroup PBFT
 * @{
 */

namespace pillar_chain {
class PillarChainManager;
}

class FullNode;
class PeriodData;
class VoteManager;

/**
 * @brief PbftManager class is a daemon that is used to finalize a bench of directed acyclic graph (DAG) blocks by using
 * Practical Byzantine Fault Tolerance (PBFT) protocol
 *
 * According to paper "ALGORAND AGREEMENT Super Fast and Partition Resilient Byzantine Agreement
 * (https://eprint.iacr.org/2018/377.pdf)", implement PBFT manager for finalizing DAG blocks.
 *
 * There are 5 states in one PBFT round: proposal state, filter state, certify state, finish state, and finish polling
 * state.
 * - Proposal state: PBFT step 1. Generate a PBFT block and propose a vote on the block hash
 * - Filter state: PBFT step 2. Identify a leader block from all received proposed blocks for the current period by
 * using minimum Verifiable Random Function (VRF) output. Soft vote at the leader block hash. In filter state, don’t
 * need check vote value correction.
 * - Certify state: PBFT step 3. If receive enough soft votes, cert vote at the value. If receive enough cert votes,
 * finalize the PBFT block and push it to PBFT chain.
 * - Finish state: Happens at even number steps from step 4. Next vote at finishing value for the current PBFT round. If
 * node receives enough next voting votes, PBFT goes to next round.
 * - Finish polling state: Happens at odd number steps from step 5. Next vote at finishing value for the current PBFT
 * round. If node receives enough next voting votes, PBFT goes to next round.
 *
 * PBFT timing: All players keep a timer clock. The timer clock will reset to 0 at every new PBFT round. That doesn’t
 * require all players clocks to be synchronized; it only requires that they have the same clock speed.
 * - Proposal state: Reset clock to 0
 * - Filter state: Start at clock 2 lambda time
 * - Certify state: Start after filter state, clock is between 2 lambda and 4 lambda duration
 * - Finish state: Start at 4 lambda time, until receive enough next voting votes to go to next round
 * - Finish polling state: Start after first finish state. If node receives enough next voting votes within 2 lambda
 * duration, PBFT will go to next round. Otherwise that will go back to Finish state.
 */
class PbftManager {
 public:
  class EligibleWallets {
   public:
    EligibleWallets(const std::vector<WalletConfig> &wallets);
    void updateWalletsEligibility(PbftPeriod period, const std::shared_ptr<final_chain::FinalChain> &final_chain);
    const std::vector<std::pair<bool, WalletConfig>> &getWallets(PbftPeriod current_pbft_period) const;

    /*
     * @return period, for which wallets eligibility was updated
     */
    PbftPeriod getWalletsEligiblePeriod() const;

   private:
    // Period, for which wallets eligibility is set
    PbftPeriod period_{0};
    std::vector<std::pair<bool /* dpos eligibility flag */, WalletConfig>> wallets_;
  };

  struct ProposedBlockData {
    std::shared_ptr<PbftBlock> pbft_block;
    std::vector<std::shared_ptr<PbftVote>> reward_votes;
    std::shared_ptr<PbftVote> vote;
  };

  using time_point = std::chrono::system_clock::time_point;

 public:
  PbftManager(const FullNodeConfig &conf, std::shared_ptr<DbStorage> db,
              rust::Box<rustaxa::BridgePbftManagerRuntime> pbft_manager_runtime, std::shared_ptr<PbftChain> pbft_chain,
              std::shared_ptr<VoteManager> vote_mgr, std::shared_ptr<DagManager> dag_mgr,
              std::shared_ptr<TransactionManager> trx_mgr, std::shared_ptr<final_chain::FinalChain> final_chain,
              std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_mgr);
  ~PbftManager();
  PbftManager(const PbftManager &) = delete;
  PbftManager(PbftManager &&) = delete;
  PbftManager &operator=(const PbftManager &) = delete;
  PbftManager &operator=(PbftManager &&) = delete;

  /**
   * @brief Set network as a weak pointer
   * @param network a weak pinter
   */
  void setNetwork(std::weak_ptr<Network> network);

  /**
   * @brief Start PBFT daemon
   */
  void start();

  /**
   * @brief Stop PBFT daemon
   */
  void stop();

  /**
   * @brief Run PBFT daemon
   */
  void run();

  /**
   * @brief Initial PBFT states when node start PBFT
   */
  void initialState();

  /**
   * @brief Get a DAG block period number
   * @param hash DAG block hash
   * @return true with DAG block period number if the DAG block has been finalized. Otherwise return false
   */
  std::pair<bool, PbftPeriod> getDagBlockPeriod(const blk_hash_t &hash);

  /**
   * @brief Get current PBFT period number
   * @return current PBFT period
   */
  PbftPeriod getPbftPeriod() const;

  /**
   * @brief Get current PBFT round number
   * @return current PBFT round
   */
  PbftRound getPbftRound() const;

  /**
   * @brief Get PBFT round & period number
   * @return legacy public API order <PBFT round, PBFT period>
   */
  std::pair<PbftRound, PbftPeriod> getPbftRoundAndPeriod() const;

  /**
   * @brief Get PBFT step number
   * @return PBFT step
   */
  PbftStep getPbftStep() const;

  /**
   * @brief Set PBFT round number
   * @param round PBFT round
   */
  void setPbftRound(PbftRound round);

  /**
   * @brief Set PBFT step
   * @param pbft_step PBFT step
   */
  void setPbftStep(PbftStep pbft_step);

  /**
   * @brief Generate PBFT block, push into unverified queue, and broadcast to peers
   * @param propose_period
   * @param prev_blk_hash previous PBFT block hash
   * @param anchor_hash proposed DAG pivot block hash for finalization
   * @param order_hash the hash of all DAG blocks include in the PBFT block
   * @param final_chain_hash FinalChain hash selected by Rust proposal construction
   * @param extra_data optional extra_data
   * @param eligible_wallets list of eligible wallets to generate pbft lock for propose_period
   * @return optional<ProposedBlockData>
   */
  std::optional<ProposedBlockData> generatePbftBlock(PbftPeriod propose_period, const blk_hash_t &prev_blk_hash,
                                                     const blk_hash_t &anchor_hash, const blk_hash_t &order_hash,
                                                     const blk_hash_t &final_chain_hash,
                                                     const std::optional<PbftBlockExtraData> &extra_data,
                                                     const std::vector<WalletConfig> &eligible_wallets);

  /**
   * @brief Get current total DPOS votes count
   * @return current total DPOS votes count if successful, otherwise (due to non-existent data for pbft_period) empty
   * optional
   */
  std::optional<uint64_t> getCurrentDposTotalVotesCount() const;

  /**
   * @brief Get current node DPOS votes count
   * @return node current DPOS votes count if successful, otherwise (due to non-existent data for pbft_period) empty
   * optional
   */
  std::optional<uint64_t> getCurrentNodeVotesCount() const;

  /**
   * @brief Get PBFT blocks synced period
   * @return PBFT blocks synced period
   */
  PbftPeriod pbftSyncingPeriod() const;

  struct PbftSyncEgressPayload {
    dev::bytes period_data_rlp;
    bool attach_reward_votes{false};
  };

  /**
   * @brief Load the Rust-owned PBFT sync egress payload and sidecar attachment decision.
   *
   * Inputs are packet-position facts and temporary C++ reward-vote sidecar facts from the network handler. Rust loads
   * the canonical PeriodData bytes from rustaxa-storage and decides whether the caller should attach the reward-vote
   * bundle. Transport, packet encoding, and vote sidecar materialization remain outside this storage boundary.
   */
  PbftSyncEgressPayload getPbftSyncEgressPayload(PbftPeriod period, bool last_block, bool pbft_chain_synced,
                                                 bool reward_votes_present, PbftPeriod reward_votes_period) const;

  /**
   * @brief Enable or disable PBFT sync snapshot creation through the Rust-mode PBFT manager boundary.
   *
   * Network sync uses this method when entering or leaving deep PBFT sync so packet handlers do not own or route
   * storage handles. The storage shim remains the temporary snapshot lifecycle adapter until snapshot creation itself
   * is migrated to Rust storage.
   */
  void setPbftSyncSnapshotCreationEnabled(bool enabled);

  /**
   * @brief Get PBFT blocks syncing queue size
   * @return PBFT syncing queue size
   */
  size_t periodDataQueueSize() const;

  /**
   * @brief Returns true if queue is empty
   * @return
   */
  bool periodDataQueueEmpty() const;

  /**
   * @brief Push synced period data in syncing queue
   * @param block synced period data from peer
   * @param current_block_cert_votes cert votes for PeriodData pbft block period
   * @param node_id peer node ID
   */
  void periodDataQueuePush(PeriodData &&period_data, dev::p2p::NodeID const &node_id,
                           std::vector<std::shared_ptr<PbftVote>> &&current_block_cert_votes);

  /**
   * @brief Get last pbft block hash from queue or if queue empty, from chain
   * @return last block hash
   */
  blk_hash_t lastPbftBlockHashFromQueueOrChain();

  /**
   * @brief Calculate DAG blocks ordering hash
   * @param dag_block_hashes DAG blocks hashes
   * @return DAG blocks ordering hash
   */
  static blk_hash_t calculateOrderHash(const std::vector<blk_hash_t> &dag_block_hashes);

  /**
   * @brief Calculate DAG blocks ordering hash
   * @param dag_blocks DAG blocks
   * @return DAG blocks ordering hash
   */
  static blk_hash_t calculateOrderHash(const std::vector<std::shared_ptr<DagBlock>> &dag_blocks);

  /**
   * @brief Reorder transactions data if DAG reordering caused transactions with same sender to have nonce in incorrect
   * order. Reordering is deterministic so that same order is produced on any node on any platform
   * @param transactions transactions to reorder
   */
  static void reorderTransactions(SharedTransactions &transactions);

  /**
   * @brief Check a block weight of gas estimation
   * @param dag_blocks dag blocks
   * @param period period
   * @return true if total weight of gas estimation is less or equal to gas limit. Otherwise return false
   */
  bool checkBlockWeight(const std::vector<std::shared_ptr<DagBlock>> &dag_blocks, PbftPeriod period) const;

  blk_hash_t getLastPbftBlockHash();

  /**
   * @brief Push proposed block into the proposed_blocks_ in case it is not there yet
   *
   * @param proposed_block
   */
  void processProposedBlock(const std::shared_ptr<PbftBlock> &proposed_block);

  /**
   * @brief Get a proposed PBFT block based on specified period and block hash
   * @param period
   * @param block_hash
   * @return std::shared_ptr<PbftBlock>
   */
  std::shared_ptr<PbftBlock> getPbftProposedBlock(PbftPeriod period, const blk_hash_t &block_hash) const;

  /**
   * @brief Get PBFT committee size
   * @return PBFT committee size
   */
  size_t getPbftCommitteeSize() const { return kGenesisConfig.pbft.committee_size; }

  /**
   * @brief Test/enforce broadcastVotes() to actually send votes
   */
  void testBroadcastVotesFunctionality();

  /**
   * @brief Check PBFT blocks syncing queue. If there are synced PBFT blocks in queue, push it to PBFT chain
   */
  void pushSyncedPbftBlocksIntoChain();

  // DPOS
  /**
   * @brief wait for DPOS period finalization
   */
  void waitForPeriodFinalization();

  /**
   * @brief Validates pbft block extra data presence + pillar votes presence based on pbft block number and ficus hf
   * block number
   *
   * @note See validatePbftBlockExtraData description, it is called inside
   * @param period_data
   * @return true if valid, otherwise false
   */
  bool validatePillarDataInPeriodData(const PeriodData &period_data) const;

  /**
   * @brief Gossips vote to the other peers
   *
   * @param vote
   * @param voted_block
   * @param rebroadcast
   */
  void gossipVote(const std::shared_ptr<PbftVote> &vote, const std::shared_ptr<PbftBlock> &voted_block,
                  bool rebroadcast = false);

  /**
   * @param period
   * @param node_addr
   * @return true if node can participate in consensus - is dpos eligible to vote and create blocks for specified period
   */
  bool canParticipateInConsensus(PbftPeriod period, const addr_t &node_addr) const;

  /**
   * @return proposed blocks ordered by period
   */
  std::map<PbftPeriod, std::vector<std::shared_ptr<PbftBlock>>> getProposedBlocks() const;

  /**
   * @return pbft deadline time - max time to finalize the block in provided period
   */
  std::chrono::milliseconds getPbftDeadline() const;

 private:
  /**
   * @brief Broadcast or rebroadcast 2t+1 soft/reward/previous round next votes + all own votes if needed
   */
  void broadcastVotes();

  /**
   * @brief If node receives 2t+1 cert votes for some valid block and pushes it to the chain, advance period to + 1.
   * @return true if PBFT period advanced, otherwise false
   */
  bool advancePeriod();

  /**
   * Applies the Rust-planned PBFT manager period-advance effect script.
   *
   * Inputs are the just-finalized PBFT-chain size and the accepted Rust reset-consensus transition plan. The method
   * executes only the temporary compatibility effects still owned by the shim: timers, wallet eligibility, vote
   * cleanup, and proposed-block cleanup. Rust remains the source of ordering and runtime snapshot updates, and every
   * completed action is reported back to Rust before the final period cursor is committed.
   *
   * Returns false when Rust rejects the plan or resulting runtime period snapshot.
   */
  bool applyRustPlannedAdvancePeriod_(PbftPeriod finalized_chain_size);
  bool applyRustPlannedAdvancePeriod_(PbftPeriod finalized_chain_size,
                                      const rustaxa::PbftManagerTransitionPlan &transition_plan);

  /**
   * @brief Check if there is 2t+1 cert votes for some valid block, if yes - push it into the chain
   * @return true if new cert voted block was pushed into the chain, otherwise false
   */
  bool tryPushCertVotesBlock();

  /**
   * @brief Resets pbft consensus: current pbft round is set to round, step is set to the beginning value
   * @param round
   */
  void resetPbftConsensus(PbftRound round);

  /**
   * @param start_time
   * @return elapsed time in ms from provided start_time
   */
  std::chrono::milliseconds elapsedTimeInMs(const time_point &start_time);

  /**
   * @brief Time to sleep for PBFT protocol
   */
  void sleep_();

  /**
   * @brief Set PBFT filter state
   */
  void setFilterState_();

  /**
   * @brief Set PBFT certify state
   */
  void setCertifyState_();

  /**
   * @brief Set PBFT finish state
   */
  void setFinishState_();

  /**
   * @brief Set PBFT finish polling state
   */
  void setFinishPollingState_();

  /**
   * @brief Set back to PBFT finish state from PBFT finish polling state
   */
  void loopBackFinishState_();

  /**
   * @brief PBFT proposal state. PBFT step 1. Propose a PBFT block and place a proposal vote on the block hash.
   */
  void proposeBlock_();

  /**
   * @brief PBFT filter state. PBFT step 2. Identify a leader block from all received proposed blocks for the current
   * period, and place a soft vote at the leader block hash.
   */
  void identifyBlock_();

  /**
   * @brief PBFT certify state. PBFT step 3. If receive enough soft votes and pass verification, place a cert vote at
   * the value.
   */
  void certifyBlock_();

  /**
   * @brief PBFT finish state. Happens at even number steps from step 4. Place a next vote at finishing value for the
   * current PBFT round.
   */
  void firstFinish_();

  /**
   * @brief PBFT finish polling state: Happens at odd number steps from step 5. Place a next vote at finishing value for
   * the current PBFT round.
   */
  void secondFinish_();

  /**
   * @brief Generate and place(gossip) vote
   *
   * @param pbft_block
   * @param vote_type
   * @param period
   * @param round
   * @param step
   * @param block_hash
   * @return
   */
  bool genAndPlaceVote(PbftVoteTypes vote_type, PbftPeriod period, PbftRound round, PbftStep step,
                       const blk_hash_t &block_hash, std::shared_ptr<PbftBlock> pbft_block = nullptr);

  /**
   * @brief Executes one Rust-planned PBFT state-action vote intent through the live C++ vote manager.
   *
   * @param vote_type vote type selected by the Rust state-action planner
   * @param period PBFT period for the vote
   * @param round PBFT round for the vote
   * @param step PBFT step for the vote
   * @param block_hash target block hash, or null block hash for null next-votes
   * @param pbft_block admitted target block when the vote is for a concrete PBFT block
   * @param action_context stable log context for the consuming manager phase
   * @param next_vote_status optional manager status bit to persist after successful next-vote placement
   * @return true when at least one eligible local wallet produced and stored the vote
   */
  bool placeStateActionVote(PbftVoteTypes vote_type, PbftPeriod period, PbftRound round, PbftStep step,
                            const blk_hash_t &block_hash, std::shared_ptr<PbftBlock> pbft_block,
                            std::string_view action_context,
                            std::optional<PbftMgrStatus> next_vote_status = std::nullopt);

  /**
   * @brief Generate propose vote for provided block place (gossip) it
   *
   * @param proposed_block
   * @param reward_votes for proposed_block
   * @return true if successful, otherwise false
   */
  bool genAndPlaceProposeVote(const std::shared_ptr<PbftBlock> &proposed_block,
                              std::vector<std::shared_ptr<PbftVote>> &&reward_votes);

  /**
   * @brief Gossips newly generated own vote to the other peers
   *
   * @param vote
   * @param voted_block
   */
  void gossipNewOwnVote(const std::shared_ptr<PbftVote> &vote, const std::shared_ptr<PbftBlock> &voted_block);

  /**
   * @brief Gossips newly generated own votes bundle to the other peers
   *
   * @param votes
   */
  void gossipNewOwnVotesBundle(const std::vector<std::shared_ptr<PbftVote>> &votes);

  /**
   * @brief Propose a new PBFT block
   * @return optional<ProposedBlockData> in case new block was proposed, otherwise empty optional
   */
  std::optional<ProposedBlockData> proposePbftBlock();

  /**
   * @brief Creates pbft block extra data
   *
   * @param pbft_period
   * @return std::optional<PbftBlockExtraData>
   */
  std::optional<PbftBlockExtraData> createPbftBlockExtraData(PbftPeriod pbft_period) const;

  /**
   * @brief Validates pbft block. It checks if:
   *        - pbft_block's previous pbft block hash == node's latest finalized pbft block
   *        - node has all DAG blocks with correct ordering,
   *        - node has all reward votes
   *        - total gas estimation is not greater than gas limit
   * @param pbft_block PBFT block
   * @return true if pbft block is valid, otherwise false
   */
  bool validatePbftBlock(const std::shared_ptr<PbftBlock> &pbft_block) const;

  /**
   * @brief Validates pbft block final chain hash.
   * @param pbft_block PBFT block
   * @return validation result
   */
  PbftStateRootValidation validateFinalChainHash(const std::shared_ptr<PbftBlock> &pbft_block) const;

  /**
   * @brief Validates pbft block extra data presence:
   *        - checks if extra data is present or not based on pbft block number and ficus hf block number
   *        - checks if pillar block hash is present on not during specific pbft periods
   *
   * @param pbft_block
   * @return true if valid, otherwise false
   */
  bool validatePbftBlockExtraData(const std::shared_ptr<PbftBlock> &pbft_block) const;

  /**
   * @brief If there are enough certify votes, push the vote PBFT block in PBFT chain
   * @param pbft_block PBFT block
   * @param current_round_cert_votes certify votes
   * @return true if push a new PBFT block in chain
   */
  bool pushCertVotedPbftBlockIntoChain_(const std::shared_ptr<PbftBlock> &pbft_block,
                                        std::vector<std::shared_ptr<PbftVote>> &&current_round_cert_votes);

  /**
   * @brief Final chain executes a finalized PBFT block
   * @param period_data PBFT block, cert votes, DAG blocks, and transactions
   * @param finalized_dag_blk_hashes DAG blocks hashes
   * @param blocks_per_year - expected number of blocks generated per year based on pbft block dynamic lambda
   * @param synchronous_processing wait for block finalization to finish
   */
  void finalize_(PeriodData &&period_data, std::vector<h256> &&finalized_dag_blk_hashes, uint32_t blocks_per_year,
                 bool synchronous_processing = false);

  /**
   * @brief Push a new PBFT block into the PBFT chain
   * @param period_data PBFT block, cert votes for previous period, DAG blocks, and transactions
   * @param cert_votes cert votes for pbft block period
   * @return true if push a new PBFT block into the PBFT chain
   */
  bool pushPbftBlock_(PeriodData &&period_data, std::vector<std::shared_ptr<PbftVote>> &&cert_votes);

  /**
   * @brief Get valid proposed pbft block. It will retrieve block from proposed_blocks and then validate it if not
   *        already validated
   *
   * @param proposed_blocks
   * @param period
   * @param block_hash
   * @return valid proposed pbft block or nullptr
   */
  std::shared_ptr<PbftBlock> getValidPbftProposedBlock(ProposedBlocks &proposed_blocks, PbftPeriod period,
                                                       const blk_hash_t &block_hash);

  /**
   * @brief Resolves a Rust-planned PBFT state-action block hash through the Rust proposed-block admission planner.
   *
   * @param period PBFT period of the requested block
   * @param block_hash PBFT block hash selected by a Rust state-action plan
   * @param action_context stable log context for the manager phase consuming the block
   * @return admitted proposed block or nullptr when Rust admission rejects or lookup/validation fails
   */
  std::shared_ptr<PbftBlock> admitStateActionPbftBlock(const rustaxa::PbftManagerStateActionEffect &effect,
                                                       std::string_view action_context);

  /**
   * @brief Process synced PBFT blocks if PBFT syncing queue is not empty
   * @return period data with cert votes for the current period
   */
  std::optional<std::pair<PeriodData, std::vector<std::shared_ptr<PbftVote>>>> processPeriodData();

  /**
   * @brief Validates PBFT block cert votes against Rust-owned sync queue block metadata
   * @param block_period PBFT block period carried by Rust queue metadata
   * @param block_hash PBFT block hash carried by Rust queue metadata
   * @param cert_votes
   *
   * @return true if there is enough(2t+1) votes and all of them are valid, otherwise false
   */
  bool validatePbftBlockCertVotes(PbftPeriod block_period, const blk_hash_t &block_hash,
                                  const std::vector<std::shared_ptr<PbftVote>> &cert_votes) const;

  /**
   @brief Validates PBFT block pillar votes
   *
   * @param period_data
   * @return
   */
  bool validatePbftBlockPillarVotes(const PeriodData &period_data) const;

  /**
   * @brief Prints all votes generated by node in current round
   */
  void printVotingSummary() const;

  /**
   * @brief Creates pillar block (and pillar vote in case node is eligible to vote & is not syncing)
   *
   * @param period
   */
  void processPillarBlock(PbftPeriod period);

  /**
   * @param round
   * @return lambda based on specified round
   */
  uint32_t getRoundLambda(PbftRound round) const;

  std::atomic<bool> stopped_ = true;

  // Multiple proposed pbft blocks could have same dag block anchor at same period so this cache improves retrieval of
  // dag block order for specific anchor
  mutable std::unordered_map<blk_hash_t, std::vector<std::shared_ptr<DagBlock>>> anchor_dag_block_order_cache_;

  std::unique_ptr<std::thread> daemon_;
  // Compatibility edge kept only for network/EVM/public materialization and lifecycle wiring while the shim owns those
  // boundaries.
  std::shared_ptr<DbStorage> db_;
  // Rust-owned PBFT manager runtime. Remaining C++ fields below are compatibility mirrors or executor/public API
  // materialization caches; they must not be used as Rust-mode protocol authority.
  mutable std::optional<rust::Box<rustaxa::BridgePbftManagerRuntime>> pbft_manager_runtime_;
  std::shared_ptr<PbftChain> pbft_chain_;
  std::shared_ptr<VoteManager> vote_mgr_;
  std::shared_ptr<DagManager> dag_mgr_;
  std::weak_ptr<Network> network_;
  std::shared_ptr<TransactionManager> trx_mgr_;
  std::shared_ptr<final_chain::FinalChain> final_chain_;
  std::shared_ptr<pillar_chain::PillarChainManager> pillar_chain_mgr_;

  const uint32_t kSyncingThreadPoolSize;
  std::shared_ptr<util::ThreadPool>
      sync_thread_pool_;  // Thread pool used for transaction sender retrieval in syncing blocks

  const std::chrono::milliseconds kMaxExponentialLambda{60000};  // [ms], max lambda is 1 minute

  // Runtime scalar mirrors hydrated from Rust snapshots for temporary executor/public API compatibility.
  uint32_t rounds_count_dynamic_lambda_{0};  // rounds count per cacti_hf.lambda_change_interval blocks
  uint32_t dynamic_lambda_{0};               // [ms] - dynamic lambda that can be anywhere between <500ms, 1500ms>
  std::chrono::milliseconds current_round_lambda_{0};  // [ms] - current round lambda

  const uint32_t kBroadcastVotesLambdaTime = 20;
  const uint32_t kRebroadcastVotesLambdaTime = 60;
  // Broadcast counter mirrors kept for vote gossip/rebroadcast executor compatibility.
  uint32_t broadcast_votes_counter_ = 1;
  uint32_t rebroadcast_votes_counter_ = 1;
  uint32_t broadcast_reward_votes_counter_ = 1;
  uint32_t rebroadcast_reward_votes_counter_ = 1;

  // Cursor mirrors hydrated from Rust snapshots for temporary executor/public API compatibility.
  PbftStates state_ = value_proposal_state;
  std::atomic<PbftRound> round_ = 1;
  PbftStep step_ = 1;

  // Temporary object materialization cache for vote-generation and network/public API boundaries. Rust owns the
  // cert-voted metadata and canonical payload.
  std::optional<std::shared_ptr<PbftBlock>> cert_voted_block_for_round_{};

  // Summary logging cache for votes created by this node in the current round.
  std::map<blk_hash_t, std::vector<PbftStep>> current_round_broadcasted_votes_;

  // Local wall-clock executor timers. Rust owns transition planning; C++ owns sleeping and scheduling effects.
  time_point current_round_start_datetime_;
  time_point current_period_start_datetime_;
  time_point second_finish_step_start_datetime_;
  std::chrono::milliseconds next_step_time_ms_{0};

  // Runtime boolean mirrors hydrated from Rust snapshots for temporary executor/public API compatibility.
  bool executed_pbft_block_ = false;
  bool already_next_voted_value_ = false;
  bool already_next_voted_null_block_hash_ = false;
  bool go_finish_state_ = false;
  bool loop_back_finish_state_ = false;
  // Pillar-vote placement guard kept at the temporary pillar/network executor boundary.
  PbftPeriod last_placed_pillar_vote_period_ = 0;

  // Used to avoid cyclic logging in voting steps that are called repeatedly
  bool printSecondFinishStepInfo_ = true;
  bool printCertStepInfo_ = true;

  const blk_hash_t dag_genesis_block_hash_;

  const GenesisConfig &kGenesisConfig;

  std::condition_variable stop_cv_;
  std::mutex stop_mtx_;

  /**
   * Live C++ sidecar for Rust-owned PBFT sync queue metadata.
   *
   * Rust owns entry ids, period admission, size, cleanup, and pop-source decisions inside `pbft_manager_runtime_`.
   * The shim keeps live legacy objects here only until `PeriodData` and peer identity payloads are ported.
   */
  struct QueuedPeriodDataPayload {
    uint64_t entry_id = 0;
    PeriodData period_data;
    dev::p2p::NodeID node_id;
  };

  struct PoppedPeriodDataPayload {
    PeriodData period_data;
    std::vector<std::shared_ptr<PbftVote>> cert_votes;
    dev::p2p::NodeID node_id;
    uint64_t period = 0;
    blk_hash_t block_hash;
    blk_hash_t prev_block_hash;
    blk_hash_t pivot_hash;
    blk_hash_t final_chain_hash;
    std::vector<vote_hash_t> reward_vote_hashes;
    std::vector<bytes> pillar_vote_rlps;
    std::vector<bytes> transaction_rlps;
    std::vector<trx_hash_t> dag_transaction_hashes;
    std::vector<trx_hash_t> period_data_transaction_hashes;
    std::vector<rustaxa::TransactionManagerVerifyNotFinalizedRuntimeFact> period_data_transaction_identities;
    bool previous_cert_votes_present = false;
    bool previous_cert_first_vote_has_weight = false;
    bool pillar_votes_present = false;
    bool extra_data_present = false;
    bool extra_data_pillar_block_hash_present = false;
  };

  QueuedPeriodDataPayload popQueuedPeriodDataPayload(uint64_t expected_entry_id);
  PoppedPeriodDataPayload popPeriodDataQueueWithMetadata();
  void clearPeriodDataQueueSidecars();
  void cleanOldPeriodDataQueueSidecars(uint64_t period);

  mutable std::shared_mutex period_data_queue_access_;
  std::deque<QueuedPeriodDataPayload> period_data_queue_payloads_;
  uint64_t next_period_data_queue_entry_id_{1};

  // Proposed blocks based on received propose votes
  ProposedBlocks proposed_blocks_;

  // Wallets with flag if they are/are not dpos eligible for specified period
  EligibleWallets eligible_wallets_;

  LOG_OBJECTS_DEFINE
};

/** @}*/

}  // namespace taraxa
