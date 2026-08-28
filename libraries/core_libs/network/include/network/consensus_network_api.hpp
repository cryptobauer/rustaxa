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

/** Read-only application-owned PBFT-sync lifecycle projection for tarcap executors. */
struct PbftSyncStatus {
  bool active = false;
  bool deep_syncing = false;
  uint64_t generation = 0;
  bool has_peer = false;
  std::array<uint8_t, 64> peer_id{};
  bool has_last_peer = false;
  std::array<uint8_t, 64> last_peer_id{};
  uint64_t target_chain_size = 0;
  uint64_t current_period = 0;
  uint64_t request_period = 0;
  uint64_t started_at_ms = 0;
  uint64_t last_activity_ms = 0;
  uint64_t elapsed_ms = 0;
  uint64_t inactive_for_ms = 0;
  uint64_t start_count = 0;
  uint64_t stop_count = 0;
  uint64_t inactivity_count = 0;
  uint64_t disconnect_count = 0;
  uint8_t last_stop_reason = 0;
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

/** Canonical consensus packet plus immutable network and protocol facts for one native ingress operation. */
struct ConsensusPacketRequest {
  uint32_t transport_lane = 0;
  std::array<uint8_t, 64> peer_id{};
  uint64_t peer_pbft_chain_size = 0;
  uint64_t source_payload_id = 0;
  std::vector<uint8_t> packet_rlp;
  uint64_t current_period = 0;
  uint64_t current_round = 0;
  uint64_t current_step = 0;
  uint64_t max_future_period_delta = 0;
  uint64_t max_future_round_delta = 0;
  uint64_t max_future_step_delta = 0;
  bool validate_max_round_step = false;
  bool can_request_pbft_sync = false;
  bool can_request_next_votes_sync = false;
};

/** Plain physical effect selected by the native consensus network service. */
struct ConsensusTransportEffect {
  uint64_t effect_id = 0;
  uint64_t source_payload_id = 0;
  uint32_t transport_lane = 0;
  uint8_t kind = 0;
  std::array<uint8_t, 64> peer_id{};
  uint32_t packet_kind = 0;
  std::vector<uint8_t> payload_bytes;
  uint8_t object_kind = 0;
  std::array<uint8_t, 32> object_hash{};
  uint8_t sync_kind = 0;
  uint64_t sync_start = 0;
  uint8_t reason_code = 0;
  uint64_t dependency_id = 0;
  uint64_t period = 0;
  uint64_t round = 0;
};

/** Typed physical execution result reported against the exact native effect id. */
struct ConsensusTransportExecutionResult {
  bool success = true;
  std::string diagnostic;
};

/** Narrow executor retained by tarcap for physical transport and peer/socket bookkeeping. */
struct ConsensusTransportExecutor {
  std::function<ConsensusTransportExecutionResult(const ConsensusTransportEffect&)> execute;
};

/** Canonical object identity requested by native egress policy from socket-owned peer-known caches. */
struct ConsensusEgressProbe {
  uint32_t probe_id = 0;
  uint8_t object_kind = 0;
  std::array<uint8_t, 32> object_hash{};
};

/** Immutable physical peer facts for one prepared application-owned egress operation. */
struct ConsensusEgressPeerSnapshot {
  std::array<uint8_t, 64> peer_id{};
  bool syncing = false;
  std::vector<uint32_t> known_probe_ids;
};

using ConsensusEgressPeerSnapshotProvider =
    std::function<std::vector<ConsensusEgressPeerSnapshot>(const std::vector<ConsensusEgressProbe>&)>;

/** Canonical application egress input; native policy constructs complete packets and selects exact target peers. */
struct ConsensusEgressRequest {
  uint8_t family = 0;
  uint32_t transport_lane = 0;
  uint64_t source_payload_id = 0;
  std::array<uint8_t, 64> source_peer_id{};
  bool rebroadcast = false;
  std::array<uint8_t, 32> object_hash{};
  std::vector<uint8_t> payload_bytes;
  std::vector<uint8_t> related_payload_bytes;
};

/** Compact terminal summary for one canonical vote or pillar-vote packet operation. */
struct ConsensusPacketOutcome {
  uint8_t status = 0;
  bool malicious = false;
  uint32_t queued_effect_count = 0;
  size_t accepted_count = 0;
  size_t duplicate_count = 0;
  size_t rejected_count = 0;
  bool has_peer_pbft_chain_size = false;
  uint64_t peer_pbft_chain_size = 0;
  std::string error_code;
  std::vector<uint8_t> egress_payload_bytes;
};

/** Canonical peer snapshot and local cursor used to select and start one native PBFT-sync generation. */
struct PbftSyncStartRequest {
  bool start = false;
  uint64_t now_ms = 0;
  uint64_t local_pbft_synced_period = 0;
  uint64_t local_pbft_chain_size = 0;
  std::vector<ConsensusPeerCandidate> candidates;
};

struct PbftSyncStartOutcome {
  uint8_t status = 0;
  std::string error_code;
  bool started = false;
  bool has_peer = false;
  std::array<uint8_t, 64> peer_id{};
  uint64_t peer_pbft_chain_size = 0;
  uint64_t request_period = 0;
  uint64_t generation = 0;
  bool deep_syncing = false;
  bool enable_snapshot_creation = false;
};

/** Identity and retained-history facts for one initial status admission. */
struct InitialStatusRequest {
  uint64_t local_chain_id = 0;
  uint64_t peer_chain_id = 0;
  std::array<uint8_t, 32> expected_genesis_hash{};
  std::array<uint8_t, 32> peer_genesis_hash{};
  uint64_t local_pbft_synced_period = 0;
  uint64_t peer_pbft_chain_size = 0;
  bool peer_is_light_node = false;
  uint64_t peer_light_node_history = 0;
};

struct InitialStatusOutcome {
  uint8_t status = 0;
  std::string error_code;
  bool accept_peer = false;
  bool disconnect_peer = false;
};

/** Local public facts used to plan one canonical initial or periodic status packet. */
struct StatusEgressRequest {
  bool initial = false;
  uint64_t local_chain_id = 0;
  std::array<uint8_t, 32> genesis_hash{};
  uint32_t node_major_version = 0;
  uint32_t node_minor_version = 0;
  uint32_t node_patch_version = 0;
  bool is_light_node = false;
  uint64_t light_node_history = 0;
  uint64_t local_pbft_chain_size = 0;
  uint64_t local_pbft_round = 0;
  uint64_t local_dag_level = 0;
};

struct StatusEgressOutcome {
  uint8_t status = 0;
  std::string error_code;
  uint64_t peer_pbft_chain_size = 0;
  uint64_t peer_pbft_round = 0;
  uint64_t peer_dag_level = 0;
  bool peer_syncing = false;
  bool include_initial_data = false;
  uint64_t chain_id = 0;
  std::array<uint8_t, 32> genesis_hash{};
  uint32_t node_major_version = 0;
  uint32_t node_minor_version = 0;
  uint32_t node_patch_version = 0;
  bool is_light_node = false;
  uint64_t light_node_history = 0;
};

/** Accepted periodic status facts used for application-owned sync follow-up selection. */
struct StatusFollowupRequest {
  std::array<uint8_t, 64> peer_id{};
  uint64_t local_pbft_synced_period = 0;
  uint64_t local_pbft_period = 0;
  uint64_t local_pbft_round = 0;
  uint64_t peer_pbft_chain_size = 0;
  uint64_t peer_pbft_period = 0;
  uint64_t peer_pbft_round = 0;
  bool peer_dag_synced = false;
};

struct StatusFollowupOutcome {
  bool request_pbft_sync = false;
  bool request_pending_dag_blocks = false;
  bool request_next_votes = false;
  uint64_t next_votes_period = 0;
  uint64_t next_votes_round = 0;
  uint64_t sync_generation = 0;
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

/** Response-source identity checked against application-owned PBFT sync state. */
enum class PbftSyncResponseSource : uint8_t { kActive, kMostRecent };

/** Typed application-owned PBFT-sync lifecycle and continuation decision. */
struct PbftSyncLifecycleOutcome {
  bool accepted = false;
  bool active = false;
  bool stopped = false;
  bool expired = false;
  bool restart_sync = false;
  bool retry = false;
  bool request_next = false;
  bool request_pending_dag_if_idle = false;
  bool deep_syncing = false;
  uint64_t generation = 0;
  std::string error_code;
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

/** Terminal native decision for one canonical transaction packet. */
struct TransactionPacketOutcome {
  uint8_t status = 0;
  uint32_t queued_effect_count = 0;
  size_t admitted_transaction_count = 0;
  std::string error_code;
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

/** Physical peer-known and exact packet-send leaves for native DAG packet ingress. */
struct DagPacketExecutor {
  std::function<void(const std::array<uint8_t, 64>&, const std::array<uint8_t, 32>&)> mark_transaction_known;
  std::function<void(const std::array<uint8_t, 64>&, const std::array<uint8_t, 32>&)> mark_dag_block_known;
  ConsensusEgressPeerSnapshotProvider gossip_snapshot;
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
 * is synchronized in Rust, and each operation retains its private lane lock across physical transport and exact-id
 * acknowledgement.
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

  /** Correlates a response peer with the active or most-recent native sync generation. */
  PbftSyncLifecycleOutcome admitPbftSyncSource(const std::array<uint8_t, 64>& peer_id,
                                               PbftSyncResponseSource source) const;

  /** Records progress for one exact native sync generation. */
  PbftSyncLifecycleOutcome recordPbftSyncActivity(uint64_t now_ms, uint64_t generation,
                                                  const std::array<uint8_t, 64>& peer_id) const;

  /** Stops one exact native sync generation for a stable stop reason. */
  PbftSyncLifecycleOutcome stopPbftSync(uint64_t generation, const std::array<uint8_t, 64>& peer_id,
                                        uint8_t reason) const;

  /** Applies selected-peer disconnect recovery to one exact generation. */
  PbftSyncLifecycleOutcome handlePbftSyncDisconnect(uint64_t generation, const std::array<uint8_t, 64>& peer_id) const;

  /** Applies inactivity policy to one timer-observed generation. */
  PbftSyncLifecycleOutcome tickPbftSync(uint64_t now_ms, uint64_t generation) const;

  /** Atomically plans queue-drain completion, restart, and pending-DAG follow-up. */
  PbftSyncLifecycleOutcome completePbftSync(uint64_t now_ms, uint64_t generation,
                                            const std::array<uint8_t, 64>& peer_id, uint64_t sync_queue_size) const;

  /** Plans stop, retry, or next-request work after the last block in a response. */
  PbftSyncLifecycleOutcome planPbftSyncLastBlock(uint64_t now_ms, uint64_t generation,
                                                 const std::array<uint8_t, 64>& peer_id, uint64_t syncing_period,
                                                 uint64_t finalized_period, uint64_t remote_period,
                                                 uint64_t sync_level_size) const;

  /** Plans bounded delayed continuation while C++ retains only timer scheduling and physical send. */
  PbftSyncLifecycleOutcome planDelayedPbftSync(uint64_t now_ms, uint64_t generation,
                                               const std::array<uint8_t, 64>& peer_id, uint64_t syncing_period,
                                               uint64_t finalized_period, uint64_t sync_level_size,
                                               uint32_t retry_count, uint64_t retry_delay_ms) const;

  /** Atomically selects a peer and optionally starts one native PBFT-sync generation. */
  PbftSyncStartOutcome beginPbftSync(const PbftSyncStartRequest& request) const;

  /** Admits immutable identity/history facts from one initial status packet. */
  InitialStatusOutcome admitInitialStatus(const InitialStatusRequest& request) const;

  /** Plans canonical initial or periodic status payload fields from native lifecycle state. */
  StatusEgressOutcome planStatusEgress(const StatusEgressRequest& request) const;

  /** Selects exact sync follow-up operations after one accepted periodic status packet. */
  StatusFollowupOutcome planStatusFollowup(const StatusFollowupRequest& request) const;

  /**
   * Constructs and routes one application-originated consensus packet family.
   *
   * Native preparation decodes canonical inputs, retains complete packets and exposes only exact object probes. The
   * provider atomically snapshots socket-owned syncing/known facts for those probes; native planning then emits exact
   * target sends and dependent known marks. A failed snapshot or plan cancels the one-shot preparation before
   * returning.
   */
  ConsensusPacketOutcome routeConsensusEgress(const ConsensusEgressRequest& request,
                                              const ConsensusEgressPeerSnapshotProvider& peer_snapshot_provider,
                                              const ConsensusTransportExecutor& executor);

  /** Routes one complete canonical transaction packet and executes only peer-known and physical gossip leaves. */
  TransactionPacketOutcome ingestTransactionPacket(uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id,
                                                    uint64_t source_payload_id, const std::vector<uint8_t>& packet_rlp,
                                                    const FullNodeConfig& config,
                                                    const ConsensusTransportExecutor& executor);

  /** Serves one canonical get-DAG-sync request from application-owned DAG/transaction bytes. */
  GetDagSyncOutcome serveGetDagSyncRequest(uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id,
                                           uint64_t source_payload_id, bool request_allowed,
                                           const std::vector<uint8_t>& request_rlp, const GetDagSyncExecutor& executor);

  /** Selects canonical non-finalized hashes in Rust and sends one exact Get-DAG-sync request. */
  PendingDagBlocksOutcome requestPendingDagBlocks(uint32_t transport_lane, uint64_t local_pbft_syncing_period,
                                                  const std::vector<ConsensusPeerCandidate>& candidates,
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

  /** Routes one canonical PBFT-vote packet and executes its exact physical transport effects. */
  ConsensusPacketOutcome ingestPbftVotePacket(const ConsensusPacketRequest& request, const FullNodeConfig& config,
                                              const ConsensusTransportExecutor& executor);

  /** Routes one canonical PBFT-vote bundle and executes all member effects on the source lane. */
  ConsensusPacketOutcome ingestPbftVotesBundlePacket(const ConsensusPacketRequest& request,
                                                     const FullNodeConfig& config,
                                                     const ConsensusTransportExecutor& executor);

  /** Routes one canonical pillar-vote packet and executes its exact physical transport effects. */
  ConsensusPacketOutcome ingestPillarVotePacket(const ConsensusPacketRequest& request,
                                                const ConsensusTransportExecutor& executor);

  /** Routes one canonical pillar-vote bundle and executes all member effects on the source lane. */
  ConsensusPacketOutcome ingestPillarVotesBundlePacket(const ConsensusPacketRequest& request,
                                                       const ConsensusTransportExecutor& executor);

  /**
   * Selects a serviceable max-chain peer from network-owned facts.
   *
   * The returned peer id identifies the selected candidate; no value means
   * that Rust found no serviceable peer. FFI conversion remains private to
   * this wrapper.
   */
  std::optional<std::array<uint8_t, 64>> selectMaxChainPeer(
      uint64_t local_pbft_syncing_period, const std::vector<ConsensusPeerCandidate>& candidates) const;

  /** Returns a side-effect-free sync snapshot through the client-oriented query API. */
  PbftSyncStatus pbftSyncStatus(uint64_t now_ms) const;

 private:
  struct TransportDrainOutcome {
    size_t successful_effect_count = 0;
    size_t failed_effect_count = 0;
  };
  std::unique_lock<std::mutex> lockTransportLane(uint32_t transport_lane);
  TransportDrainOutcome drainAndExecuteTransportEffects(uint32_t transport_lane, uint64_t source_payload_id,
                                                        bool source_scoped,
                                                        const ConsensusTransportExecutor& executor);
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
