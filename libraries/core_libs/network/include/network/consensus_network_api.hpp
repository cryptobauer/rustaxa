#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <vector>

#include "common/types.hpp"

namespace rustaxa {
class BridgeConsensusNetworkApi;
class BridgeConsensusApplication;
}  // namespace rustaxa

namespace taraxa {
class ConsensusApplication;
class DagBlock;
struct FullNodeConfig;
using SharedConsensusApplication = std::shared_ptr<ConsensusApplication>;
}  // namespace taraxa

namespace taraxa::final_chain {
class FinalChain;
}

namespace taraxa::network {

/** Read-only live consensus facts supplied by App without exposing its native root or private executor. */
struct ConsensusLiveStatus {
  uint64_t period = 0;
  uint64_t round = 0;
  uint64_t step = 0;
  uint64_t syncing_period = 0;
  size_t sync_queue_size = 0;
  bool sync_queue_empty = true;
};

using ConsensusLiveStatusProvider = std::function<ConsensusLiveStatus()>;

/** Potentially expensive DPoS vote diagnostics, sampled only by periodic node statistics. */
struct ConsensusVoteStatus {
  std::optional<uint64_t> total_dpos_votes;
  std::optional<uint64_t> node_dpos_votes;
};

using ConsensusVoteStatusProvider = std::function<ConsensusVoteStatus()>;

/** Best-effort post-commit observers for native network admissions. */
struct ConsensusNetworkObservers {
  std::function<void(const std::vector<uint8_t>&)> dag_block_observed;
  std::function<void(const trx_hash_t&)> transaction_observed;
};

/** Plain network-owned facts for one max-chain peer candidate. */
struct ConsensusPeerCandidate {
  std::array<uint8_t, 64> peer_id{};
  uint64_t pbft_chain_size = 0;
  uint64_t dag_level = 0;
  bool is_light_node = false;
  uint64_t light_node_history = 0;
  bool peer_dag_synced = false;
  bool peer_dag_syncing = false;
  bool dag_sync_allowed = false;
};

/** Physical tarcap leaves for one native pillar-vote bundle response. */
struct PillarVotesBundleExecutor {
  std::function<bool(const std::vector<uint8_t>&)> send_bundle;
  std::function<void(const std::array<uint8_t, 32>&)> mark_vote_known;
  std::function<void(uint8_t)> report_peer;
  std::function<void()> disconnect_peer;
};

/** Terminal native decision for one pillar-vote bundle request. */
struct PillarVotesBundleRequestOutcome {
  uint8_t status = 0;
  uint32_t queued_effect_count = 0;
  std::string error_code;
};

/** Physical tarcap leaves for one native Get-PBFT-sync response plan. */
struct PbftSyncRequestExecutor {
  std::function<bool(uint32_t, const std::vector<uint8_t>&)> send_packet;
  std::function<void()> clear_peer_syncing;
  std::function<void(uint8_t)> report_peer;
  std::function<void()> disconnect_peer;
};

/** Terminal native decision for one Get-PBFT-sync request. */
struct PbftSyncRequestOutcome {
  uint8_t status = 0;
  uint32_t queued_effect_count = 0;
  std::string error_code;
};

/** Physical tarcap leaf for one native previous-round next-vote response. */
struct PbftNextVotesBundleExecutor {
  std::function<bool(const std::vector<uint8_t>&)> send_bundle;
};

/** Terminal native decision for one previous-round next-vote request. */
struct PbftNextVotesBundleRequestOutcome {
  uint8_t status = 0;
  uint32_t queued_effect_count = 0;
  std::string error_code;
};

/** Terminal native decision for one proposed-PBFT-block bundle. */
struct PbftBlocksBundleOutcome {
  uint8_t status = 0;
  std::string error_code;
};

/** Transport-facing action selected by native PBFT-sync ingress. */
enum class PbftSyncIngressAction : uint8_t {
  kContinue,
  kDuplicate,
  kSyncComplete,
  kDrop,
  kStopSyncing,
  kMalicious,
  kQueueRejected,
};

/** Terminal native decision and compact transport facts for one PBFT-sync packet. */
struct PbftSyncIngressOutcome {
  PbftSyncIngressAction action = PbftSyncIngressAction::kDrop;
  std::string error_code;
  std::array<uint8_t, 32> block_hash{};
  uint64_t block_period = 0;
  uint64_t max_dag_level = 0;
  bool last_block = false;
  bool current_cert_present = false;
};

/** Narrow C++ transaction facts for one native double-voting slashing effect. */
struct PbftSyncSlashingTransaction {
  uint8_t status = 0;
  size_t wallet_index = 0;
  std::array<uint8_t, 32> nonce{};
  std::array<uint8_t, 20> contract_address{};
  std::array<uint8_t, 32> value{};
  uint64_t gas_limit = 0;
  std::vector<uint8_t> call_data;
};

/** Canonical fields for the retained PBFT-vote slashing transaction leaf. */
struct PbftVoteSlashingTransaction {
  uint8_t status = 0;
  std::array<uint8_t, 32> proof_hash{};
  size_t wallet_index = 0;
  std::array<uint8_t, 32> nonce{};
  std::array<uint8_t, 20> contract_address{};
  std::array<uint8_t, 32> value{};
  uint64_t gas_limit = 0;
  std::vector<uint8_t> call_data;
};

/** Concrete-EVM account facts used only by native PBFT-sync slashing admission. */
struct PbftSyncSlashingSubmitterFact {
  size_t wallet_index = 0;
  std::array<uint8_t, 32> nonce{};
  std::array<uint8_t, 32> balance{};
};

/** Narrow physical executor for a native double-voting slashing transaction. */
struct PbftSyncIngressExecutor {
  std::function<bool(const PbftSyncSlashingTransaction&)> submit_slashing_transaction;
};

/** Physical tarcap leaves for native transaction-packet admission effects. */
struct TransactionPacketExecutor {
  std::function<void(const std::array<uint8_t, 64>&, const std::array<uint8_t, 32>&)> mark_transaction_known;
  std::function<bool(const std::vector<uint8_t>&, const std::vector<std::array<uint8_t, 64>>&)> gossip_packet;
};

/** Terminal native decision for one canonical transaction packet. */
struct TransactionPacketOutcome {
  uint8_t status = 0;
  uint32_t queued_effect_count = 0;
  size_t admitted_transaction_count = 0;
  std::string error_code;
};

/** One physical peer and its bounded known subset for native periodic transaction gossip. */
struct TransactionGossipPeer {
  std::array<uint8_t, 64> peer_id{};
  std::vector<std::array<uint8_t, 32>> known_hashes;
};

/** Physical leaves for exact native periodic transaction-gossip effects. */
struct TransactionGossipExecutor {
  std::function<bool(const std::array<uint8_t, 64>&, const std::vector<uint8_t>&)> send_packet;
  std::function<void(const std::array<uint8_t, 64>&, const std::array<uint8_t, 32>&)> mark_transaction_known;
};

/** Physical send leaf and terminal outcome for one get-DAG-sync request. */
struct GetDagSyncExecutor {
  std::function<bool(const std::vector<uint8_t>&, uint64_t, uint64_t)> send_response;
};

struct GetDagSyncOutcome {
  uint8_t status = 0;
  uint32_t queued_effect_count = 0;
  std::string error_code;
};

/** Physical send leaf for one native pending-DAG request. */
struct PendingDagBlocksExecutor {
  std::function<bool(const std::array<uint8_t, 64>&, const std::vector<uint8_t>&, uint64_t)> send_request;
};

/** Terminal native decision for one pending-DAG request. */
struct PendingDagBlocksOutcome {
  uint8_t status = 0;
  uint32_t queued_effect_count = 0;
  std::string error_code;
};

/** One physical peer candidate for exact native DAG-block fanout. */
struct DagGossipPeer {
  std::array<uint8_t, 64> peer_id{};
  bool syncing = false;
  bool known_block = false;
};

/** Physical peer-known and exact packet-send leaves for native DAG packet ingress. */
struct DagPacketExecutor {
  std::function<void(const std::array<uint8_t, 64>&, const std::array<uint8_t, 32>&)> mark_transaction_known;
  std::function<void(const std::array<uint8_t, 64>&, const std::array<uint8_t, 32>&)> mark_dag_block_known;
  std::function<std::vector<DagGossipPeer>(const std::array<uint8_t, 32>&)> gossip_candidates;
  std::function<bool(const std::array<uint8_t, 64>&, const std::vector<uint8_t>&)> send_packet;
};

/** Compact post-admission facts for one native DAG block. */
struct DagBlockAdmissionOutcome {
  std::array<uint8_t, 32> block_hash{};
  uint64_t block_level = 0;
  bool accepted = false;
  bool duplicate = false;
  uint32_t reject_code = 0;
};

/** Transport-owned peer facts used by Rust to select the exact DAG rejection action. */
struct DagBlockPeerFacts {
  bool peer_dag_synced = false;
  bool dag_sync_allowed = false;
  bool transactions_dropped = false;
  bool pending_dag_request = false;
  bool local_pbft_syncing = false;
};

/** Terminal native decision for one DAG-block packet. */
struct DagBlockPacketOutcome {
  uint8_t status = 0;
  uint32_t queued_effect_count = 0;
  /** Native action: none, ignore, full sync, pending sync, disconnect, or malicious. */
  uint8_t rejection_action = 0;
  std::string error_code;
  std::optional<DagBlockAdmissionOutcome> admission;
};

/** Terminal native decision for one DAG-sync packet. */
struct DagSyncPacketOutcome {
  uint8_t status = 0;
  uint32_t queued_effect_count = 0;
  std::string error_code;
  uint64_t request_period = 0;
  uint64_t response_period = 0;
  std::vector<DagBlockAdmissionOutcome> blocks;
};

/**
 * Owns the main-only Rust consensus network facade for one Network instance.
 *
 * Construction retains the application root and clones its network service for effect dispatch; it cannot create a
 * second protocol runtime or queue. Destruction releases the adapter and its shared root ownership. Native state access
 * is synchronized in Rust, while callers retain the lane lock across physical transport and acknowledgement.
 */
class ConsensusNetworkApi final {
 public:
  ConsensusNetworkApi(SharedConsensusApplication consensus_application,
                      std::shared_ptr<final_chain::FinalChain> final_chain, ConsensusNetworkObservers observers = {});
  ~ConsensusNetworkApi();

  ConsensusNetworkApi(const ConsensusNetworkApi&) = delete;
  ConsensusNetworkApi(ConsensusNetworkApi&&) = delete;
  ConsensusNetworkApi& operator=(const ConsensusNetworkApi&) = delete;
  ConsensusNetworkApi& operator=(ConsensusNetworkApi&&) = delete;

  /** Returns the live Rust facade owned by this wrapper. */
  rustaxa::BridgeConsensusNetworkApi& api() noexcept;
  /** Returns the live Rust facade owned by this wrapper. */
  const rustaxa::BridgeConsensusNetworkApi& api() const noexcept;

  /**
   * Locks one transport lane across effect drain, physical execution, and acknowledgement.
   *
   * Callers for the same lane are serialized in drain order, while distinct lanes may execute concurrently. The
   * returned lock releases the lane when destroyed.
   */
  std::unique_lock<std::mutex> lockTransportLane(uint32_t transport_lane);

  /**
   * Routes and executes one pillar-vote bundle request on its transport lane.
   *
   * Rust owns schedule validation, vote lookup/validation, chunking, effect
   * ordering, and dependency acknowledgement. The callbacks perform only
   * packet transport and peer bookkeeping; a failed chunk send suppresses its
   * marks without preventing later independent chunks.
   */
  PillarVotesBundleRequestOutcome servePillarVotesBundleRequest(uint32_t transport_lane,
                                                                const std::array<uint8_t, 64>& peer_id, uint64_t period,
                                                                const std::array<uint8_t, 32>& pillar_block_hash,
                                                                uint64_t source_payload_id,
                                                                const PillarVotesBundleExecutor& executor);

  /**
   * Routes and executes one canonical Get-PBFT-sync request on its transport lane.
   *
   * Rust validates the request, reads canonical period data and native proposed
   * blocks, selects reward-vote attachment, and orders all response effects.
   * The callbacks retain only packet sealing and peer-state operations. Version
   * 5 intentionally emits no proposed-block bundles; versions other than 5 and
   * 6 are rejected by the native request planner.
   */
  PbftSyncRequestOutcome servePbftSyncRequest(uint32_t tarcap_version, const std::array<uint8_t, 64>& peer_id,
                                              const std::vector<uint8_t>& request_rlp, uint64_t source_payload_id,
                                              const PbftSyncRequestExecutor& executor);

  /**
   * Routes and executes one previous-round next-vote request on its transport lane.
   *
   * Rust owns request eligibility, the live PBFT cursor snapshot, verified-vote
   * lookup, bundle validation, chunking, and send ordering. The caller supplies
   * only peer request fields and physical packet transport.
   */
  PbftNextVotesBundleRequestOutcome servePbftNextVotesBundleRequest(uint32_t transport_lane,
                                                                    const std::array<uint8_t, 64>& peer_id,
                                                                    uint64_t peer_period, uint64_t peer_round,
                                                                    uint64_t source_payload_id,
                                                                    const PbftNextVotesBundleExecutor& executor);

  /**
   * Admits one latest-tarcap proposed-block bundle into native PBFT state.
   *
   * Rust owns raw decoding, relevance and author checks, FinalChain DPoS
   * queries, and storage-first proposal publication. The caller retains only
   * peer-level error handling.
   */
  PbftBlocksBundleOutcome admitPbftBlocksBundle(const std::vector<uint8_t>& packet_rlp, uint64_t source_payload_id);

  /**
   * Publishes one canonical signed proposed block selected by a native network effect.
   *
   * Rust decodes and verifies the block identity, pivot, and period before the
   * application root performs its storage-first publication. The return value
   * is false only when the same live proposal was already present; malformed
   * bytes and persistence failures propagate without C++ block materialization.
   */
  bool publishProposedBlockEffect(const std::vector<uint8_t>& canonical_signed_block_rlp);

  /**
   * Admits one original PBFT-sync packet through the native application root.
   *
   * Rust owns exact decoding, deterministic prechecks, sequential weighted vote
   * admission, reward selection, and queue mutation. The executor is invoked
   * synchronously only for a typed slashing transaction effect; its Boolean
   * insertion result is reported before native ingress advances to the next
   * vote. Transport facts remain available for legacy peer bookkeeping without
   * materializing consensus objects in C++.
   */
  PbftSyncIngressOutcome admitPbftSyncPacket(const std::vector<uint8_t>& packet_rlp, uint64_t source_payload_id,
                                             const std::array<uint8_t, 64>& source_peer_id,
                                             const std::vector<PbftSyncSlashingSubmitterFact>& slashing_submitters,
                                             const PbftSyncIngressExecutor& executor);

  /** Routes one complete canonical transaction packet and executes only peer-known and physical gossip leaves. */
  TransactionPacketOutcome ingestTransactionPacket(uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id,
                                                   uint64_t source_payload_id, const std::vector<uint8_t>& packet_rlp,
                                                   bool rebroadcast, const FullNodeConfig& config,
                                                   const TransactionPacketExecutor& executor);

  /** Returns the bounded native candidate hash set used to sample physical peer-known caches. */
  std::vector<std::array<uint8_t, 32>> transactionGossipCandidateHashes() const;

  /** Plans and executes exact per-peer periodic transaction packets from the native queue. */
  TransactionPacketOutcome planTransactionGossip(uint32_t transport_lane,
                                                 const std::vector<TransactionGossipPeer>& peers,
                                                 const TransactionGossipExecutor& executor);

  /** Serves one canonical get-DAG-sync request from application-owned DAG/transaction bytes. */
  GetDagSyncOutcome serveGetDagSyncRequest(uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id,
                                           uint64_t source_payload_id, bool request_allowed,
                                           const std::vector<uint8_t>& request_rlp, const GetDagSyncExecutor& executor);

  /** Selects canonical non-finalized hashes in Rust and sends one exact Get-DAG-sync request. */
  PendingDagBlocksOutcome requestPendingDagBlocks(uint32_t transport_lane, uint64_t local_pbft_syncing_period,
                                                  const ConsensusPeerCandidate& explicit_peer,
                                                  const PendingDagBlocksExecutor& executor);

  /** Routes one canonical DAG-block packet through native admission and executes only physical network leaves. */
  DagBlockPacketOutcome ingestDagBlockPacket(uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id,
                                             uint64_t source_payload_id, const std::vector<uint8_t>& packet_rlp,
                                             bool rebroadcast, const DagBlockPeerFacts& peer_facts,
                                             const FullNodeConfig& config, const DagPacketExecutor& executor);

  /** Routes one canonical DAG-sync packet through native sequential admission and physical peer-known leaves. */
  DagSyncPacketOutcome ingestDagSyncPacket(uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id,
                                           uint64_t source_payload_id, const std::vector<uint8_t>& packet_rlp,
                                           const FullNodeConfig& config, const DagPacketExecutor& executor);

  /** Reports the concrete transaction-insertion result for one native vote slashing effect. */
  bool reportPbftVoteSlashingSubmission(const std::array<uint8_t, 32>& proof_hash, bool transaction_inserted);

  /**
   * Signs, inserts, and reports one native PBFT-vote slashing transaction.
   *
   * The native effect selects the configured wallet and supplies canonical
   * nonce/value/calldata. C++ retains secret custody and concrete transaction
   * insertion. Invalid effects throw before signing; the returned value is the
   * native acknowledgement of the actual insertion result.
   */
  bool executePbftVoteSlashingTransaction(const PbftVoteSlashingTransaction& effect, const FullNodeConfig& config);

  /** Signs and submits one PBFT-sync slashing transaction through native transaction admission. */
  bool executePbftSyncSlashingTransaction(const PbftSyncSlashingTransaction& effect, const FullNodeConfig& config);

  /**
   * Selects a serviceable max-chain peer from network-owned facts.
   *
   * The returned peer id identifies the selected candidate; no value means
   * that Rust found no serviceable peer. FFI conversion remains private to
   * this wrapper.
   */
  std::optional<std::array<uint8_t, 64>> selectMaxChainPeer(
      uint64_t local_pbft_syncing_period, const std::vector<ConsensusPeerCandidate>& candidates) const;

 private:
  bool submitSlashingTransaction(size_t wallet_index, const std::array<uint8_t, 32>& nonce,
                                 const std::array<uint8_t, 20>& contract_address, const std::array<uint8_t, 32>& value,
                                 uint64_t gas_limit, const std::vector<uint8_t>& call_data,
                                 const FullNodeConfig& config);
  class Impl;
  std::unique_ptr<Impl> impl_;
};

/** Shared lifetime used by Network and every Rust-mode tarcap consumer. */
using ConsensusNetworkApiShared = std::shared_ptr<ConsensusNetworkApi>;

}  // namespace taraxa::network
