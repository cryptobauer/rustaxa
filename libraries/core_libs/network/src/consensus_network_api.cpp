#include "network/consensus_network_api.hpp"

#include <algorithm>
#include <atomic>
#include <mutex>
#include <stdexcept>
#include <unordered_map>
#include <utility>

#ifdef RUSTAXA_ENABLE
#include "consensus/consensus_application.hpp"
#include "consensus/consensus_host_ports.hpp"
#include "final_chain/final_chain.hpp"
#include "rustaxa-bridge/application_host_ffi.rs.h"
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/transaction.hpp"

namespace taraxa::network {

namespace {

constexpr uint8_t kEffectResultOk = 0;
constexpr uint8_t kEffectResultFailed = 1;
constexpr uint8_t kEffectSendPacket = 0;
constexpr uint8_t kEffectGossipPacket = 1;
constexpr uint8_t kEffectMarkPeerKnown = 2;
constexpr uint8_t kEffectReportPeer = 4;
constexpr uint8_t kEffectDisconnectPeer = 5;
constexpr uint8_t kEffectClearPeerSyncing = 9;
constexpr uint8_t kObjectPillarVote = 5;
constexpr uint8_t kObjectTransaction = 2;
constexpr uint8_t kObjectDagBlock = 3;
constexpr uint32_t kPacketPillarVotesBundle = 15;
constexpr uint32_t kPacketPbftSync = 11;
constexpr uint32_t kPacketPbftBlocksBundle = 16;
constexpr uint32_t kPacketPbftVotesBundle = 3;
constexpr uint32_t kPacketTransaction = 7;
constexpr uint32_t kPacketDagBlock = 5;
constexpr uint32_t kPacketDagSync = 6;
constexpr uint32_t kPacketGetDagSync = 12;
constexpr uint32_t kEffectDrainBudget = 1024;
constexpr uint8_t kPbftSyncIngressContinue = 0;
constexpr uint8_t kPbftSyncIngressDuplicate = 1;
constexpr uint8_t kPbftSyncIngressComplete = 2;
constexpr uint8_t kPbftSyncIngressDrop = 3;
constexpr uint8_t kPbftSyncIngressStop = 4;
constexpr uint8_t kPbftSyncIngressMalicious = 5;
constexpr uint8_t kPbftSyncIngressQueueRejected = 6;
constexpr uint8_t kPbftSyncIngressAwaitingSlashing = 7;

SharedConsensusApplication requireConsensusApplication(SharedConsensusApplication consensus_application) {
  if (!consensus_application) {
    throw std::invalid_argument("Consensus network API requires a shared consensus application");
  }
  return consensus_application;
}

rust::Vec<uint8_t> toRustBytes(const dev::bytes& value) {
  rust::Vec<uint8_t> out;
  out.reserve(value.size());
  for (const auto byte : value) {
    out.push_back(byte);
  }
  return out;
}

template <typename Value>
std::array<uint8_t, 32> toBridgeU256(const Value& value) {
  std::array<uint8_t, 32> out{};
  const auto bytes = dev::toBigEndian(value);
  if (bytes.size() > out.size()) {
    throw std::runtime_error("u256 value exceeds 32 bytes");
  }
  std::copy(bytes.begin(), bytes.end(), out.begin() + static_cast<std::ptrdiff_t>(out.size() - bytes.size()));
  return out;
}

}  // namespace

class ConsensusNetworkApi::Impl final {
 public:
  Impl(SharedConsensusApplication consensus_application, std::shared_ptr<final_chain::FinalChain> final_chain,
       ConsensusNetworkObservers observers)
      : consensus_application(requireConsensusApplication(std::move(consensus_application))),
        final_chain(std::move(final_chain)),
        external_evm(this->final_chain),
        api(rustaxa::create_consensus_network_api(this->consensus_application->service())),
        observers(std::move(observers)) {}

  SharedConsensusApplication consensus_application;
  std::shared_ptr<final_chain::FinalChain> final_chain;
  ExternalEvmPort external_evm;
  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
  ConsensusNetworkObservers observers;
  std::mutex lanes_mutex;
  std::unordered_map<uint32_t, std::unique_ptr<std::mutex>> lane_execution_mutexes;
  std::atomic<uint64_t> next_egress_payload_id{uint64_t{1} << 63};
};

ConsensusNetworkApi::ConsensusNetworkApi(SharedConsensusApplication consensus_application,
                                         std::shared_ptr<final_chain::FinalChain> final_chain,
                                         ConsensusNetworkObservers observers)
    : impl_(std::make_unique<Impl>(std::move(consensus_application), std::move(final_chain), std::move(observers))) {}
ConsensusNetworkApi::~ConsensusNetworkApi() = default;

rustaxa::BridgeConsensusNetworkApi& ConsensusNetworkApi::api() noexcept { return *impl_->api; }

const rustaxa::BridgeConsensusNetworkApi& ConsensusNetworkApi::api() const noexcept { return *impl_->api; }

PbftSyncIngressOutcome ConsensusNetworkApi::admitPbftSyncPacket(
    const std::vector<uint8_t>& packet_rlp, uint64_t source_payload_id, const std::array<uint8_t, 64>& source_peer_id,
    const std::vector<PbftSyncSlashingSubmitterFact>& slashing_submitters, const PbftSyncIngressExecutor& executor) {
  rust::Vec<rustaxa::SlashingSubmitterIdentity> native_submitters;
  native_submitters.reserve(slashing_submitters.size());
  for (const auto& submitter : slashing_submitters) {
    rustaxa::SlashingSubmitterIdentity native{};
    native.wallet_index = submitter.wallet_index;
    native.nonce = submitter.nonce;
    native.balance = submitter.balance;
    native_submitters.push_back(std::move(native));
  }
  auto step = rustaxa::pbft_service_begin_pbft_sync_ingress(
      impl_->consensus_application->service(), rust::Slice<const uint8_t>(packet_rlp.data(), packet_rlp.size()),
      source_payload_id, source_peer_id, std::move(native_submitters));
  while (step.action == kPbftSyncIngressAwaitingSlashing) {
    if (!step.has_slashing_transaction_effect || !executor.submit_slashing_transaction) {
      throw std::runtime_error("Native PBFT-sync ingress paused without an executable slashing boundary");
    }
    const auto& native_effect = step.slashing_transaction_effect;
    PbftSyncSlashingTransaction transaction{
        native_effect.status,
        native_effect.wallet_index,
        native_effect.nonce,
        native_effect.contract_address,
        native_effect.value,
        native_effect.gas_limit,
        std::vector<uint8_t>(native_effect.call_data.begin(), native_effect.call_data.end())};
    const auto transaction_inserted = executor.submit_slashing_transaction(transaction);
    step = rustaxa::pbft_service_report_pbft_sync_ingress_slashing(
        impl_->consensus_application->service(), step.slashing_transaction_effect.proof_hash, transaction_inserted);
  }

  PbftSyncIngressAction action;
  switch (step.action) {
    case kPbftSyncIngressContinue:
      action = PbftSyncIngressAction::kContinue;
      break;
    case kPbftSyncIngressDuplicate:
      action = PbftSyncIngressAction::kDuplicate;
      break;
    case kPbftSyncIngressComplete:
      action = PbftSyncIngressAction::kSyncComplete;
      break;
    case kPbftSyncIngressDrop:
      action = PbftSyncIngressAction::kDrop;
      break;
    case kPbftSyncIngressStop:
      action = PbftSyncIngressAction::kStopSyncing;
      break;
    case kPbftSyncIngressMalicious:
      action = PbftSyncIngressAction::kMalicious;
      break;
    case kPbftSyncIngressQueueRejected:
      action = PbftSyncIngressAction::kQueueRejected;
      break;
    default:
      throw std::runtime_error("Native PBFT-sync ingress returned an unknown terminal action");
  }
  return PbftSyncIngressOutcome{action,
                                static_cast<std::string>(step.error_code),
                                step.block_hash,
                                step.period,
                                step.max_dag_level,
                                step.last_block,
                                step.current_cert_present};
}

bool ConsensusNetworkApi::reportPbftVoteSlashingSubmission(const std::array<uint8_t, 32>& proof_hash,
                                                           bool transaction_inserted) {
  return impl_->consensus_application->service().pbft_service_verified_votes_report_slashing_transaction_submission(
      proof_hash, transaction_inserted);
}

bool ConsensusNetworkApi::executePbftVoteSlashingTransaction(const PbftVoteSlashingTransaction& effect,
                                                             const FullNodeConfig& config) {
  if (effect.status != 0 || effect.wallet_index >= config.wallets.size()) {
    throw std::runtime_error("Native network vote admission returned an invalid slashing transaction effect");
  }
  const auto inserted = submitSlashingTransaction(effect.wallet_index, effect.nonce, effect.contract_address,
                                                  effect.value, effect.gas_limit, effect.call_data, config);
  return reportPbftVoteSlashingSubmission(effect.proof_hash, inserted);
}

bool ConsensusNetworkApi::executePbftSyncSlashingTransaction(const PbftSyncSlashingTransaction& effect,
                                                             const FullNodeConfig& config) {
  if (effect.status != 0 || effect.wallet_index >= config.wallets.size()) {
    throw std::runtime_error("Native PBFT-sync admission returned an invalid slashing transaction effect");
  }
  return submitSlashingTransaction(effect.wallet_index, effect.nonce, effect.contract_address, effect.value,
                                   effect.gas_limit, effect.call_data, config);
}

TransactionPacketOutcome ConsensusNetworkApi::ingestTransactionPacket(uint32_t transport_lane,
                                                                      const std::array<uint8_t, 64>& peer_id,
                                                                      uint64_t source_payload_id,
                                                                      const std::vector<uint8_t>& packet_rlp,
                                                                      bool rebroadcast, const FullNodeConfig& config,
                                                                      const TransactionPacketExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  const auto last_block_number = impl_->final_chain->lastBlockNumber();
  rustaxa::NetworkTransactionPacketRequest request{};
  request.transport_lane = transport_lane;
  request.peer_id = peer_id;
  request.source_payload_id = source_payload_id;
  request.packet_rlp.reserve(packet_rlp.size());
  for (const auto byte : packet_rlp) {
    request.packet_rlp.push_back(byte);
  }
  request.expected_chain_id = config.genesis.chain_id;
  request.maximum_gas_limit = config.genesis.state.hardforks.soleirolia_hf.trx_max_gas_limit;
  request.minimum_gas_price = toBridgeU256(val_t(config.genesis.state.hardforks.soleirolia_hf.trx_min_gas_price));
  request.last_block_number = last_block_number;
  request.cornus_active = config.genesis.state.hardforks.isOnCornusHardfork(last_block_number);
  request.rebroadcast = rebroadcast;
  const auto report = rustaxa::consensus_network_ingest_transaction_packet(
      api(), impl_->consensus_application->service(), std::move(request), impl_->external_evm);

  for (const auto& member : report.transactions) {
    if (member.observe_transaction && impl_->observers.transaction_observed) {
      try {
        impl_->observers.transaction_observed(
            trx_hash_t(member.submission.transaction_hash.data(), trx_hash_t::ConstructFromPointer));
      } catch (const std::exception&) {
        // Public observation is post-commit and best-effort. It must never
        // change native admission or transport acknowledgement.
      }
    }
  }

  while (true) {
    const auto batch = api().consensus_network_drain_work(transport_lane, source_payload_id, true, kEffectDrainBudget);
    if (batch.effects.empty()) {
      break;
    }
    rust::Vec<rustaxa::NetworkEffectResult> results;
    results.reserve(batch.effects.size());
    for (const auto& effect : batch.effects) {
      rustaxa::NetworkEffectResult result{};
      result.effect_id = effect.effect_id;
      result.kind = effect.kind;
      result.peer_id = effect.peer_id;
      result.packet_kind = effect.packet_kind;
      result.object_kind = effect.object_kind;
      result.object_hash = effect.object_hash;
      result.status = kEffectResultOk;
      try {
        if (effect.kind == kEffectMarkPeerKnown && effect.object_kind == kObjectTransaction) {
          executor.mark_transaction_known(effect.peer_id, effect.object_hash);
        } else if (effect.kind == kEffectGossipPacket && effect.packet_kind == kPacketTransaction) {
          std::vector<std::array<uint8_t, 64>> excluded;
          excluded.reserve(effect.exclude_peers.size());
          for (const auto& excluded_peer : effect.exclude_peers) {
            excluded.push_back(excluded_peer.id);
          }
          if (!executor.gossip_packet(std::vector<uint8_t>(effect.payload_bytes.begin(), effect.payload_bytes.end()),
                                      excluded)) {
            throw std::runtime_error("Transaction gossip transport failed");
          }
        } else {
          throw std::runtime_error("Transaction ingress executor received an unsupported effect");
        }
      } catch (const std::exception& error) {
        result.status = kEffectResultFailed;
        result.diagnostic = error.what();
      }
      results.push_back(std::move(result));
    }
    const auto acknowledgement = api().consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0) {
      throw std::runtime_error("Network API rejected transaction executor results: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }

  return TransactionPacketOutcome{report.decision.status, report.decision.queued_effect_count,
                                  report.transactions.size(), static_cast<std::string>(report.decision.error_code)};
}

std::vector<std::array<uint8_t, 32>> ConsensusNetworkApi::transactionGossipCandidateHashes() const {
  const auto native =
      rustaxa::consensus_network_transaction_gossip_candidate_hashes(impl_->consensus_application->service());
  std::vector<std::array<uint8_t, 32>> hashes;
  hashes.reserve(native.size());
  for (const auto& hash : native) {
    hashes.push_back(hash.hash);
  }
  return hashes;
}

TransactionPacketOutcome ConsensusNetworkApi::planTransactionGossip(uint32_t transport_lane,
                                                                    const std::vector<TransactionGossipPeer>& peers,
                                                                    const TransactionGossipExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  const auto source_payload_id = impl_->next_egress_payload_id.fetch_add(1, std::memory_order_relaxed);
  rustaxa::NetworkTransactionGossipRequest request{};
  request.transport_lane = transport_lane;
  request.source_payload_id = source_payload_id;
  request.peers.reserve(peers.size());
  for (const auto& peer : peers) {
    rustaxa::NetworkTransactionGossipPeer native_peer{};
    native_peer.peer_id = peer.peer_id;
    native_peer.known_hashes.reserve(peer.known_hashes.size());
    for (const auto& hash : peer.known_hashes) {
      rustaxa::DagHash native_hash{};
      native_hash.hash = hash;
      native_peer.known_hashes.push_back(native_hash);
    }
    request.peers.push_back(std::move(native_peer));
  }
  const auto decision =
      api().consensus_network_plan_transaction_gossip(impl_->consensus_application->service(), std::move(request));
  while (true) {
    const auto batch = api().consensus_network_drain_work(transport_lane, source_payload_id, true, kEffectDrainBudget);
    if (batch.effects.empty()) {
      break;
    }
    rust::Vec<rustaxa::NetworkEffectResult> results;
    results.reserve(batch.effects.size());
    for (const auto& effect : batch.effects) {
      rustaxa::NetworkEffectResult result{};
      result.effect_id = effect.effect_id;
      result.kind = effect.kind;
      result.peer_id = effect.peer_id;
      result.packet_kind = effect.packet_kind;
      result.object_kind = effect.object_kind;
      result.object_hash = effect.object_hash;
      result.status = kEffectResultOk;
      try {
        if (effect.kind == kEffectSendPacket && effect.packet_kind == kPacketTransaction) {
          if (!executor.send_packet(effect.peer_id,
                                    std::vector<uint8_t>(effect.payload_bytes.begin(), effect.payload_bytes.end()))) {
            throw std::runtime_error("Periodic transaction transport failed");
          }
        } else if (effect.kind == kEffectMarkPeerKnown && effect.object_kind == kObjectTransaction) {
          executor.mark_transaction_known(effect.peer_id, effect.object_hash);
        } else {
          throw std::runtime_error("Periodic transaction executor received an unsupported effect");
        }
      } catch (const std::exception& error) {
        result.status = kEffectResultFailed;
        result.diagnostic = error.what();
      }
      results.push_back(std::move(result));
    }
    const auto acknowledgement = api().consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0) {
      throw std::runtime_error("Network API rejected periodic transaction executor results: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }
  return TransactionPacketOutcome{decision.status, decision.queued_effect_count, 0,
                                  static_cast<std::string>(decision.error_code)};
}

GetDagSyncOutcome ConsensusNetworkApi::serveGetDagSyncRequest(uint32_t transport_lane,
                                                              const std::array<uint8_t, 64>& peer_id,
                                                              uint64_t source_payload_id, bool request_allowed,
                                                              const std::vector<uint8_t>& request_rlp,
                                                              const GetDagSyncExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  rustaxa::NetworkGetDagSyncRequest request{};
  request.transport_lane = transport_lane;
  request.peer_id = peer_id;
  request.source_payload_id = source_payload_id;
  request.request_allowed = request_allowed;
  request.request_rlp.reserve(request_rlp.size());
  for (const auto byte : request_rlp) {
    request.request_rlp.push_back(byte);
  }
  const auto decision =
      api().consensus_network_ingest_get_dag_sync_request(impl_->consensus_application->service(), std::move(request));
  while (true) {
    const auto batch = api().consensus_network_drain_work(transport_lane, source_payload_id, true, kEffectDrainBudget);
    if (batch.effects.empty()) {
      break;
    }
    rust::Vec<rustaxa::NetworkEffectResult> results;
    results.reserve(batch.effects.size());
    for (const auto& effect : batch.effects) {
      rustaxa::NetworkEffectResult result{};
      result.effect_id = effect.effect_id;
      result.kind = effect.kind;
      result.peer_id = effect.peer_id;
      result.packet_kind = effect.packet_kind;
      result.object_kind = effect.object_kind;
      result.object_hash = effect.object_hash;
      result.status = kEffectResultOk;
      try {
        if (effect.peer_id != peer_id || effect.kind != kEffectSendPacket || effect.packet_kind != kPacketDagSync) {
          throw std::runtime_error("Get-DAG-sync executor received a mismatched effect");
        }
        if (!executor.send_response(std::vector<uint8_t>(effect.payload_bytes.begin(), effect.payload_bytes.end()),
                                    effect.sync_start, effect.period)) {
          throw std::runtime_error("DAG-sync response transport failed");
        }
      } catch (const std::exception& error) {
        result.status = kEffectResultFailed;
        result.diagnostic = error.what();
      }
      results.push_back(std::move(result));
    }
    const auto acknowledgement = api().consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0) {
      throw std::runtime_error("Network API rejected get-DAG-sync executor results: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }
  return GetDagSyncOutcome{decision.status, decision.queued_effect_count,
                           static_cast<std::string>(decision.error_code)};
}

DagBlockPacketOutcome ConsensusNetworkApi::ingestDagBlockPacket(
    uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id, uint64_t source_payload_id,
    const std::vector<uint8_t>& packet_rlp, bool rebroadcast, const DagBlockPeerFacts& peer_facts,
    const FullNodeConfig& config, const DagPacketExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  const auto last_block_number = impl_->final_chain->lastBlockNumber();
  rustaxa::NetworkDagPacketRequest request{};
  request.transport_lane = transport_lane;
  request.peer_id = peer_id;
  request.source_payload_id = source_payload_id;
  request.packet_rlp.reserve(packet_rlp.size());
  for (const auto byte : packet_rlp) {
    request.packet_rlp.push_back(byte);
  }
  request.expected_chain_id = config.genesis.chain_id;
  request.maximum_gas_limit = config.genesis.state.hardforks.soleirolia_hf.trx_max_gas_limit;
  request.minimum_gas_price = toBridgeU256(val_t(config.genesis.state.hardforks.soleirolia_hf.trx_min_gas_price));
  request.last_block_number = last_block_number;
  request.cornus_active = config.genesis.state.hardforks.isOnCornusHardfork(last_block_number);
  request.rebroadcast = rebroadcast;
  request.peer_dag_synced = peer_facts.peer_dag_synced;
  request.dag_sync_allowed = peer_facts.dag_sync_allowed;
  request.transactions_dropped = peer_facts.transactions_dropped;
  request.pending_dag_request = peer_facts.pending_dag_request;
  request.local_pbft_syncing = peer_facts.local_pbft_syncing;
  const auto report = rustaxa::consensus_network_ingest_dag_block_packet(api(), impl_->consensus_application->service(),
                                                                         std::move(request), impl_->external_evm);

  if (report.admission_found && report.admission.observe_block && impl_->observers.dag_block_observed) {
    try {
      impl_->observers.dag_block_observed(
          std::vector<uint8_t>(report.admission.block_rlp.begin(), report.admission.block_rlp.end()));
    } catch (const std::exception&) {
      // Public observation is post-commit and best-effort.
    }
  }
  uint32_t queued_effect_count = report.decision.queued_effect_count;
  if (report.admission_found && report.admission.accepted && report.admission.gossip_block && rebroadcast) {
    rustaxa::NetworkDagGossipRequest gossip{};
    gossip.transport_lane = transport_lane;
    gossip.source_payload_id = source_payload_id;
    gossip.source_peer_id = peer_id;
    gossip.block_hash = report.admission.block_hash;
    gossip.packet_rlp.reserve(packet_rlp.size());
    for (const auto byte : packet_rlp) {
      gossip.packet_rlp.push_back(byte);
    }
    if (executor.gossip_candidates) {
      const auto candidates = executor.gossip_candidates(report.admission.block_hash);
      gossip.peers.reserve(candidates.size());
      for (const auto& candidate : candidates) {
        rustaxa::NetworkDagGossipPeer native_candidate{};
        native_candidate.peer_id = candidate.peer_id;
        native_candidate.syncing = candidate.syncing;
        native_candidate.known_block = candidate.known_block;
        gossip.peers.push_back(std::move(native_candidate));
      }
    }
    const auto gossip_decision = api().consensus_network_plan_dag_block_gossip(std::move(gossip));
    queued_effect_count += gossip_decision.queued_effect_count;
  }
  while (true) {
    const auto batch = api().consensus_network_drain_work(transport_lane, source_payload_id, true, kEffectDrainBudget);
    if (batch.effects.empty()) {
      break;
    }
    rust::Vec<rustaxa::NetworkEffectResult> results;
    results.reserve(batch.effects.size());
    for (const auto& effect : batch.effects) {
      rustaxa::NetworkEffectResult result{};
      result.effect_id = effect.effect_id;
      result.kind = effect.kind;
      result.peer_id = effect.peer_id;
      result.packet_kind = effect.packet_kind;
      result.object_kind = effect.object_kind;
      result.object_hash = effect.object_hash;
      result.status = kEffectResultOk;
      try {
        if (effect.kind == kEffectMarkPeerKnown && effect.object_kind == kObjectTransaction) {
          executor.mark_transaction_known(effect.peer_id, effect.object_hash);
        } else if (effect.kind == kEffectMarkPeerKnown && effect.object_kind == kObjectDagBlock) {
          executor.mark_dag_block_known(effect.peer_id, effect.object_hash);
        } else if (effect.kind == kEffectSendPacket && effect.packet_kind == kPacketDagBlock) {
          if (!executor.send_packet(effect.peer_id,
                                    std::vector<uint8_t>(effect.payload_bytes.begin(), effect.payload_bytes.end()))) {
            throw std::runtime_error("DAG-block transport failed");
          }
        } else {
          throw std::runtime_error("DAG-block executor received an unsupported effect");
        }
      } catch (const std::exception& error) {
        result.status = kEffectResultFailed;
        result.diagnostic = error.what();
      }
      results.push_back(std::move(result));
    }
    const auto acknowledgement = api().consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0) {
      throw std::runtime_error("Network API rejected DAG-block executor results: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }
  std::optional<DagBlockAdmissionOutcome> admission;
  if (report.admission_found) {
    admission =
        DagBlockAdmissionOutcome{report.admission.block_hash, report.admission.block_level, report.admission.accepted,
                                 report.admission.duplicate, report.admission.reject_code};
  }
  return DagBlockPacketOutcome{report.decision.status, queued_effect_count, report.rejection_action,
                               static_cast<std::string>(report.decision.error_code), std::move(admission)};
}

DagSyncPacketOutcome ConsensusNetworkApi::ingestDagSyncPacket(
    uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id, uint64_t source_payload_id,
    const std::vector<uint8_t>& packet_rlp, const FullNodeConfig& config, const DagPacketExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  const auto last_block_number = impl_->final_chain->lastBlockNumber();
  rustaxa::NetworkDagPacketRequest request{};
  request.transport_lane = transport_lane;
  request.peer_id = peer_id;
  request.source_payload_id = source_payload_id;
  request.packet_rlp.reserve(packet_rlp.size());
  for (const auto byte : packet_rlp) {
    request.packet_rlp.push_back(byte);
  }
  request.expected_chain_id = config.genesis.chain_id;
  request.maximum_gas_limit = config.genesis.state.hardforks.soleirolia_hf.trx_max_gas_limit;
  request.minimum_gas_price = toBridgeU256(val_t(config.genesis.state.hardforks.soleirolia_hf.trx_min_gas_price));
  request.last_block_number = last_block_number;
  request.cornus_active = config.genesis.state.hardforks.isOnCornusHardfork(last_block_number);
  request.rebroadcast = false;
  request.local_pbft_syncing = false;
  const auto report = rustaxa::consensus_network_ingest_dag_sync_packet(api(), impl_->consensus_application->service(),
                                                                        std::move(request), impl_->external_evm);

  for (const auto& transaction : report.transactions) {
    if (transaction.observe_transaction && impl_->observers.transaction_observed) {
      try {
        impl_->observers.transaction_observed(
            trx_hash_t(transaction.submission.transaction_hash.data(), trx_hash_t::ConstructFromPointer));
      } catch (const std::exception&) {
        // Public observation is post-commit and best-effort.
      }
    }
  }
  for (const auto& block : report.blocks) {
    if (block.observe_block && impl_->observers.dag_block_observed) {
      try {
        impl_->observers.dag_block_observed(std::vector<uint8_t>(block.block_rlp.begin(), block.block_rlp.end()));
      } catch (const std::exception&) {
        // Public observation is post-commit and best-effort.
      }
    }
  }
  while (true) {
    const auto batch = api().consensus_network_drain_work(transport_lane, source_payload_id, true, kEffectDrainBudget);
    if (batch.effects.empty()) {
      break;
    }
    rust::Vec<rustaxa::NetworkEffectResult> results;
    results.reserve(batch.effects.size());
    for (const auto& effect : batch.effects) {
      rustaxa::NetworkEffectResult result{};
      result.effect_id = effect.effect_id;
      result.kind = effect.kind;
      result.peer_id = effect.peer_id;
      result.packet_kind = effect.packet_kind;
      result.object_kind = effect.object_kind;
      result.object_hash = effect.object_hash;
      result.status = kEffectResultOk;
      try {
        if (effect.kind == kEffectMarkPeerKnown && effect.object_kind == kObjectTransaction) {
          executor.mark_transaction_known(effect.peer_id, effect.object_hash);
        } else if (effect.kind == kEffectMarkPeerKnown && effect.object_kind == kObjectDagBlock) {
          executor.mark_dag_block_known(effect.peer_id, effect.object_hash);
        } else {
          throw std::runtime_error("DAG-sync executor received an unsupported effect");
        }
      } catch (const std::exception& error) {
        result.status = kEffectResultFailed;
        result.diagnostic = error.what();
      }
      results.push_back(std::move(result));
    }
    const auto acknowledgement = api().consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0) {
      throw std::runtime_error("Network API rejected DAG-sync executor results: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }
  std::vector<DagBlockAdmissionOutcome> blocks;
  blocks.reserve(report.blocks.size());
  for (const auto& block : report.blocks) {
    blocks.push_back(DagBlockAdmissionOutcome{block.block_hash, block.block_level, block.accepted, block.duplicate,
                                              block.reject_code});
  }
  return DagSyncPacketOutcome{report.decision.status,
                              report.decision.queued_effect_count,
                              static_cast<std::string>(report.decision.error_code),
                              report.request_period,
                              report.response_period,
                              std::move(blocks)};
}

PendingDagBlocksOutcome ConsensusNetworkApi::requestPendingDagBlocks(uint32_t transport_lane,
                                                                     uint64_t local_pbft_syncing_period,
                                                                     const ConsensusPeerCandidate& explicit_peer,
                                                                     const PendingDagBlocksExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  const auto source_payload_id = impl_->next_egress_payload_id.fetch_add(1, std::memory_order_relaxed);
  rustaxa::NetworkPendingDagBlocksRequestFacts facts{};
  facts.local_pbft_syncing_period = local_pbft_syncing_period;
  facts.has_explicit_peer = true;
  facts.explicit_peer.peer_id = explicit_peer.peer_id;
  facts.explicit_peer.pbft_chain_size = explicit_peer.pbft_chain_size;
  facts.explicit_peer.dag_level = explicit_peer.dag_level;
  facts.explicit_peer.is_light_node = explicit_peer.is_light_node;
  facts.explicit_peer.light_node_history = explicit_peer.light_node_history;
  facts.explicit_peer.peer_dag_synced = explicit_peer.peer_dag_synced;
  facts.explicit_peer.peer_dag_syncing = explicit_peer.peer_dag_syncing;
  facts.explicit_peer.dag_sync_allowed = explicit_peer.dag_sync_allowed;
  const auto decision = api().consensus_network_request_pending_dag_blocks(
      impl_->consensus_application->service(), transport_lane, source_payload_id, std::move(facts));

  while (true) {
    const auto batch = api().consensus_network_drain_work(transport_lane, source_payload_id, true, kEffectDrainBudget);
    if (batch.effects.empty()) {
      break;
    }
    rust::Vec<rustaxa::NetworkEffectResult> results;
    results.reserve(batch.effects.size());
    for (const auto& effect : batch.effects) {
      rustaxa::NetworkEffectResult result{};
      result.effect_id = effect.effect_id;
      result.kind = effect.kind;
      result.peer_id = effect.peer_id;
      result.packet_kind = effect.packet_kind;
      result.object_kind = effect.object_kind;
      result.object_hash = effect.object_hash;
      result.status = kEffectResultOk;
      try {
        if (effect.peer_id != explicit_peer.peer_id || effect.kind != kEffectSendPacket ||
            effect.packet_kind != kPacketGetDagSync) {
          throw std::runtime_error("Pending-DAG executor received a mismatched effect");
        }
        if (!executor.send_request(effect.peer_id,
                                   std::vector<uint8_t>(effect.payload_bytes.begin(), effect.payload_bytes.end()),
                                   effect.period)) {
          throw std::runtime_error("Pending-DAG request transport failed");
        }
      } catch (const std::exception& error) {
        result.status = kEffectResultFailed;
        result.diagnostic = error.what();
      }
      results.push_back(std::move(result));
    }
    const auto acknowledgement = api().consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0) {
      throw std::runtime_error("Network API rejected pending-DAG executor results: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }
  return PendingDagBlocksOutcome{decision.status, decision.queued_effect_count,
                                 static_cast<std::string>(decision.error_code)};
}

bool ConsensusNetworkApi::submitSlashingTransaction(size_t wallet_index, const std::array<uint8_t, 32>& nonce_bytes,
                                                    const std::array<uint8_t, 20>& contract_address,
                                                    const std::array<uint8_t, 32>& value_bytes, uint64_t gas_limit,
                                                    const std::vector<uint8_t>& call_data,
                                                    const FullNodeConfig& config) {
  const auto& wallet = config.wallets[wallet_index];
  const auto nonce = dev::fromBigEndian<u256>(dev::bytes(nonce_bytes.begin(), nonce_bytes.end()));
  const auto value = dev::fromBigEndian<u256>(dev::bytes(value_bytes.begin(), value_bytes.end()));
  const addr_t contract(contract_address.data(), addr_t::ConstructFromPointer);
  const auto gas_price_bytes =
      rustaxa::consensus_application_transaction_gas_price_bid(impl_->consensus_application->service());
  const auto gas_price = dev::fromBigEndian<u256>(dev::bytes(gas_price_bytes.begin(), gas_price_bytes.end()));
  const auto transaction = std::make_shared<Transaction>(nonce, value, gas_price, gas_limit, call_data,
                                                         wallet.node_secret, contract, config.genesis.chain_id);
  const auto last_block_number = impl_->final_chain->lastBlockNumber();
  rustaxa::PublicTransactionSubmissionRequest request{};
  request.transaction_rlp = toRustBytes(transaction->rlp());
  request.expected_chain_id = config.genesis.chain_id;
  request.maximum_gas_limit = config.genesis.state.hardforks.soleirolia_hf.trx_max_gas_limit;
  request.minimum_gas_price = toBridgeU256(val_t(config.genesis.state.hardforks.soleirolia_hf.trx_min_gas_price));
  request.last_block_number = last_block_number;
  request.cornus_active = config.genesis.state.hardforks.isOnCornusHardfork(last_block_number);
  const auto submission = rustaxa::consensus_application_submit_transaction_with_execution(
      impl_->consensus_application->service(), std::move(request), impl_->external_evm);
  return submission.accepted;
}

std::unique_lock<std::mutex> ConsensusNetworkApi::lockTransportLane(uint32_t transport_lane) {
  std::mutex* lane_mutex = nullptr;
  {
    std::lock_guard lanes_lock(impl_->lanes_mutex);
    auto& stored_mutex = impl_->lane_execution_mutexes[transport_lane];
    if (!stored_mutex) {
      stored_mutex = std::make_unique<std::mutex>();
    }
    lane_mutex = stored_mutex.get();
  }
  return std::unique_lock(*lane_mutex);
}

PillarVotesBundleRequestOutcome ConsensusNetworkApi::servePillarVotesBundleRequest(
    uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id, uint64_t period,
    const std::array<uint8_t, 32>& pillar_block_hash, uint64_t source_payload_id,
    const PillarVotesBundleExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  const auto decision = api().consensus_network_ingest_pillar_votes_bundle_request(
      transport_lane, peer_id, period, pillar_block_hash, source_payload_id);

  while (true) {
    const auto batch = api().consensus_network_drain_work(transport_lane, source_payload_id, true, kEffectDrainBudget);
    if (batch.effects.empty()) {
      break;
    }

    rust::Vec<rustaxa::NetworkEffectResult> results;
    results.reserve(batch.effects.size());
    for (const auto& effect : batch.effects) {
      rustaxa::NetworkEffectResult result{};
      result.effect_id = effect.effect_id;
      result.kind = effect.kind;
      result.peer_id = effect.peer_id;
      result.packet_kind = effect.packet_kind;
      result.object_kind = effect.object_kind;
      result.object_hash = effect.object_hash;
      result.status = kEffectResultOk;

      try {
        if (effect.peer_id != peer_id) {
          throw std::runtime_error("Pillar-vote bundle effect targets a different peer");
        }
        if (effect.kind == kEffectSendPacket && effect.packet_kind == kPacketPillarVotesBundle) {
          if (!executor.send_bundle(std::vector<uint8_t>(effect.payload_bytes.begin(), effect.payload_bytes.end()))) {
            throw std::runtime_error("Pillar-vote bundle transport send failed");
          }
        } else if (effect.kind == kEffectMarkPeerKnown && effect.object_kind == kObjectPillarVote) {
          executor.mark_vote_known(effect.object_hash);
        } else if (effect.kind == kEffectReportPeer) {
          executor.report_peer(effect.reason_code);
        } else if (effect.kind == kEffectDisconnectPeer) {
          executor.disconnect_peer();
        } else {
          throw std::runtime_error("Pillar-vote bundle executor received an unsupported effect");
        }
      } catch (const std::exception& error) {
        result.status = kEffectResultFailed;
        result.diagnostic = error.what();
      }
      results.push_back(std::move(result));
    }

    const auto acknowledgement = api().consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0) {
      throw std::runtime_error("Network API rejected pillar-vote bundle executor results: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }

  return PillarVotesBundleRequestOutcome{decision.status, decision.queued_effect_count,
                                         static_cast<std::string>(decision.error_code)};
}

PbftSyncRequestOutcome ConsensusNetworkApi::servePbftSyncRequest(uint32_t tarcap_version,
                                                                 const std::array<uint8_t, 64>& peer_id,
                                                                 const std::vector<uint8_t>& request_rlp,
                                                                 uint64_t source_payload_id,
                                                                 const PbftSyncRequestExecutor& executor) {
  auto lane_lock = lockTransportLane(tarcap_version);
  rustaxa::NetworkGetPbftSyncRequest request{};
  request.tarcap_version = tarcap_version;
  request.peer_id = peer_id;
  request.request_rlp.reserve(request_rlp.size());
  for (const auto byte : request_rlp) {
    request.request_rlp.push_back(byte);
  }
  request.source_payload_id = source_payload_id;
  const auto decision = api().consensus_network_ingest_get_pbft_sync_request(std::move(request));

  while (true) {
    const auto batch = api().consensus_network_drain_work(tarcap_version, source_payload_id, true, kEffectDrainBudget);
    if (batch.effects.empty()) {
      break;
    }

    rust::Vec<rustaxa::NetworkEffectResult> results;
    results.reserve(batch.effects.size());
    for (const auto& effect : batch.effects) {
      rustaxa::NetworkEffectResult result{};
      result.effect_id = effect.effect_id;
      result.kind = effect.kind;
      result.peer_id = effect.peer_id;
      result.packet_kind = effect.packet_kind;
      result.object_kind = effect.object_kind;
      result.object_hash = effect.object_hash;
      result.status = kEffectResultOk;

      try {
        if (effect.peer_id != peer_id) {
          throw std::runtime_error("PBFT sync effect targets a different peer");
        }
        if (effect.kind == kEffectSendPacket &&
            (effect.packet_kind == kPacketPbftSync || effect.packet_kind == kPacketPbftBlocksBundle)) {
          if (!executor.send_packet(effect.packet_kind,
                                    std::vector<uint8_t>(effect.payload_bytes.begin(), effect.payload_bytes.end()))) {
            throw std::runtime_error("PBFT sync transport send failed");
          }
        } else if (effect.kind == kEffectClearPeerSyncing) {
          executor.clear_peer_syncing();
        } else if (effect.kind == kEffectReportPeer) {
          executor.report_peer(effect.reason_code);
        } else if (effect.kind == kEffectDisconnectPeer) {
          executor.disconnect_peer();
        } else {
          throw std::runtime_error("PBFT sync executor received an unsupported effect");
        }
      } catch (const std::exception& error) {
        result.status = kEffectResultFailed;
        result.diagnostic = error.what();
      }
      results.push_back(std::move(result));
    }

    const auto acknowledgement = api().consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0) {
      throw std::runtime_error("Network API rejected PBFT sync executor results: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }

  return PbftSyncRequestOutcome{decision.status, decision.queued_effect_count,
                                static_cast<std::string>(decision.error_code)};
}

PbftNextVotesBundleRequestOutcome ConsensusNetworkApi::servePbftNextVotesBundleRequest(
    uint32_t transport_lane, const std::array<uint8_t, 64>& peer_id, uint64_t peer_period, uint64_t peer_round,
    uint64_t source_payload_id, const PbftNextVotesBundleExecutor& executor) {
  auto lane_lock = lockTransportLane(transport_lane);
  const auto decision = api().consensus_network_ingest_pbft_next_votes_bundle_request(
      transport_lane, peer_id, peer_period, peer_round, source_payload_id);

  while (true) {
    const auto batch = api().consensus_network_drain_work(transport_lane, source_payload_id, true, kEffectDrainBudget);
    if (batch.effects.empty()) {
      break;
    }

    rust::Vec<rustaxa::NetworkEffectResult> results;
    results.reserve(batch.effects.size());
    for (const auto& effect : batch.effects) {
      rustaxa::NetworkEffectResult result{};
      result.effect_id = effect.effect_id;
      result.kind = effect.kind;
      result.peer_id = effect.peer_id;
      result.packet_kind = effect.packet_kind;
      result.object_kind = effect.object_kind;
      result.object_hash = effect.object_hash;
      result.status = kEffectResultOk;

      try {
        if (effect.peer_id != peer_id) {
          throw std::runtime_error("Next-votes bundle effect targets a different peer");
        }
        if (effect.kind != kEffectSendPacket || effect.packet_kind != kPacketPbftVotesBundle) {
          throw std::runtime_error("Next-votes bundle executor received an unsupported effect");
        }
        if (!executor.send_bundle(std::vector<uint8_t>(effect.payload_bytes.begin(), effect.payload_bytes.end()))) {
          throw std::runtime_error("Next-votes bundle transport send failed");
        }
      } catch (const std::exception& error) {
        result.status = kEffectResultFailed;
        result.diagnostic = error.what();
      }
      results.push_back(std::move(result));
    }

    const auto acknowledgement = api().consensus_network_report_effect_results(std::move(results));
    if (acknowledgement.status != 0) {
      throw std::runtime_error("Network API rejected next-votes bundle executor results: " +
                               static_cast<std::string>(acknowledgement.error_code));
    }
  }

  return PbftNextVotesBundleRequestOutcome{decision.status, decision.queued_effect_count,
                                           static_cast<std::string>(decision.error_code)};
}

PbftBlocksBundleOutcome ConsensusNetworkApi::admitPbftBlocksBundle(const std::vector<uint8_t>& packet_rlp,
                                                                   uint64_t source_payload_id) {
  rust::Vec<uint8_t> bridge_packet;
  bridge_packet.reserve(packet_rlp.size());
  for (const auto byte : packet_rlp) {
    bridge_packet.push_back(byte);
  }
  const auto decision = api().consensus_network_ingest_pbft_blocks_bundle(impl_->consensus_application->service(),
                                                                          std::move(bridge_packet), source_payload_id);
  return PbftBlocksBundleOutcome{decision.status, static_cast<std::string>(decision.error_code)};
}

bool ConsensusNetworkApi::publishProposedBlockEffect(const std::vector<uint8_t>& canonical_signed_block_rlp) {
  rust::Vec<uint8_t> bridge_block;
  bridge_block.reserve(canonical_signed_block_rlp.size());
  for (const auto byte : canonical_signed_block_rlp) {
    bridge_block.push_back(byte);
  }
  return impl_->consensus_application->service().pbft_service_publish_proposed_block_effect(std::move(bridge_block));
}

std::optional<std::array<uint8_t, 64>> ConsensusNetworkApi::selectMaxChainPeer(
    uint64_t local_pbft_syncing_period, const std::vector<ConsensusPeerCandidate>& candidates) const {
  rustaxa::NetworkPeerSelectionFacts facts{};
  facts.local_pbft_syncing_period = local_pbft_syncing_period;
  facts.candidates.reserve(candidates.size());
  for (const auto& candidate : candidates) {
    rustaxa::NetworkPbftSyncPeerCandidate bridge_candidate{};
    bridge_candidate.peer_id = candidate.peer_id;
    bridge_candidate.pbft_chain_size = candidate.pbft_chain_size;
    bridge_candidate.dag_level = candidate.dag_level;
    bridge_candidate.is_light_node = candidate.is_light_node;
    bridge_candidate.light_node_history = candidate.light_node_history;
    bridge_candidate.peer_dag_synced = candidate.peer_dag_synced;
    bridge_candidate.peer_dag_syncing = candidate.peer_dag_syncing;
    bridge_candidate.dag_sync_allowed = candidate.dag_sync_allowed;
    facts.candidates.push_back(std::move(bridge_candidate));
  }

  const auto plan = api().consensus_network_plan_max_chain_peer_selection(facts);
  if (!plan.has_peer) {
    return std::nullopt;
  }
  return plan.peer_id;
}

}  // namespace taraxa::network
#endif
