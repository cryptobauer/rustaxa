#include "consensus/consensus_host_ports.hpp"

#ifdef RUSTAXA_ENABLE

#include <libdevcrypto/Common.h>

#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdio>
#include <exception>
#include <future>
#include <mutex>
#include <stdexcept>
#include <thread>
#include <unordered_map>
#include <utility>
#include <vector>

#include "common/vrf_wrapper.hpp"
#include "config/config.hpp"
#include "consensus/consensus_application.hpp"
#include "dag/dag_block.hpp"
#include "final_chain/final_chain.hpp"
#include "network/network.hpp"
#include "pbft/pbft_block.hpp"
#include "pbft/period_data.hpp"
#include "rustaxa-bridge/application_host_ffi.rs.h"
#include "transaction/transaction.hpp"
#include "vdf/sortition.hpp"
#include "vote/pbft_vote.hpp"
#include "vote/pillar_vote.hpp"
#include "vote/votes_bundle_rlp.hpp"

namespace taraxa {

rustaxa::HostFinalChainFinalizeReport ConsensusApplication::finalize(ExternalEvmPort& external_evm,
                                                                     rustaxa::HostFinalChainFinalizeTask task) const {
  return rustaxa::consensus_application_finalize(service(), external_evm, std::move(task));
}

}  // namespace taraxa

namespace taraxa {
namespace {

constexpr uint8_t kWaitElapsed = 0;
constexpr uint8_t kWaitStopped = 1;

template <typename Report>
Report failedReport(const rustaxa::HostEffectId& effect_id, const char* error) {
  Report report{};
  report.effect_id = effect_id;
  report.succeeded = false;
  report.error_code = rust::String(error);
  return report;
}

rust::Vec<uint8_t> toRustBytes(const dev::bytes& bytes) {
  rust::Vec<uint8_t> result;
  result.reserve(bytes.size());
  for (const auto byte : bytes) {
    result.push_back(byte);
  }
  return result;
}

dev::bytes fromRustBytes(const rust::Vec<uint8_t>& bytes) { return {bytes.begin(), bytes.end()}; }

template <typename Value>
std::array<uint8_t, 32> toBridgeU256(const Value& value) {
  std::array<uint8_t, 32> out{};
  const auto bytes = dev::toBigEndian(value);
  if (bytes.size() > out.size()) {
    throw std::runtime_error("host integer exceeds 32 bytes");
  }
  std::copy(bytes.begin(), bytes.end(), out.begin() + static_cast<std::ptrdiff_t>(out.size() - bytes.size()));
  return out;
}

addr_t fromHostAddress(const std::array<uint8_t, 20>& address) {
  return addr_t(address.data(), addr_t::ConstructFromPointer);
}

rustaxa::HostTransportReport successfulTransportReport(const rustaxa::HostEffectId& effect_id) {
  rustaxa::HostTransportReport report{};
  report.effect_id = effect_id;
  report.succeeded = true;
  return report;
}

class ConsensusVdfExecutor;
class ConsensusObserverExecutor;

}  // namespace

class ConsensusProcessPort::Impl final {
 public:
  Impl() = default;
  Impl(const FullNodeConfig& config, std::shared_ptr<ConsensusApplication> application);
  ~Impl();

  void reset() {
    const std::scoped_lock lock(mutex);
    stop_requested = false;
  }

  void stop() {
    {
      const std::scoped_lock lock(mutex);
      stop_requested = true;
    }
    cv.notify_all();
  }

  mutable std::mutex mutex;
  mutable std::condition_variable cv;
  bool stop_requested{false};
  std::unique_ptr<ConsensusVdfExecutor> vdf;
  std::unique_ptr<ConsensusObserverExecutor> observer;
};

rustaxa::HostWaitReport ConsensusProcessPort::consensusWait(const rustaxa::HostWaitRequest& request) const {
  std::unique_lock lock(impl_->mutex);
  const auto stopped =
      impl_->cv.wait_for(lock, std::chrono::milliseconds(request.delay_ms), [this] { return impl_->stop_requested; });
  rustaxa::HostWaitReport report{};
  report.effect_id = request.effect_id;
  report.outcome = stopped ? kWaitStopped : kWaitElapsed;
  return report;
}

bool ConsensusProcessPort::consensusStopRequested([[maybe_unused]] uint64_t generation) const {
  const std::scoped_lock lock(impl_->mutex);
  return impl_->stop_requested;
}

uint64_t ConsensusProcessPort::consensusNowMillis() const {
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now().time_since_epoch())
          .count());
}

uint64_t ConsensusProcessPort::consensusUnixTimeSeconds() const {
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::seconds>(std::chrono::system_clock::now().time_since_epoch()).count());
}

class ConsensusSignerPort::Impl final {
 public:
  explicit Impl(const FullNodeConfig& config) : wallets(config.wallets) {}

  std::vector<WalletConfig> wallets;
};

ConsensusSignerPort::ConsensusSignerPort(const FullNodeConfig& config) : impl_(std::make_unique<Impl>(config)) {}
ConsensusSignerPort::~ConsensusSignerPort() = default;

rustaxa::HostSignReport ConsensusSignerPort::consensusSignDigest(const rustaxa::HostSignRequest& request) const {
  if (request.wallet_index >= impl_->wallets.size()) {
    return failedReport<rustaxa::HostSignReport>(request.effect_id, "SIGNER_WALLET_INDEX_OUT_OF_RANGE");
  }

  rustaxa::HostSignReport report{};
  report.effect_id = request.effect_id;
  const auto digest = dev::h256(request.digest.data(), dev::h256::ConstructFromPointer);
  report.signature = toRustBytes(dev::sign(impl_->wallets[request.wallet_index].node_secret, digest).asBytes());
  report.succeeded = true;
  return report;
}

rustaxa::HostVrfReport ConsensusSignerPort::consensusProveVrf(const rustaxa::HostVrfRequest& request) const {
  if (request.wallet_index >= impl_->wallets.size()) {
    return failedReport<rustaxa::HostVrfReport>(request.effect_id, "VRF_WALLET_INDEX_OUT_OF_RANGE");
  }

  const auto& wallet = impl_->wallets[request.wallet_index];
  const auto message = fromRustBytes(request.message);
  const auto proof = vrf_wrapper::getVrfProof(wallet.vrf_secret, message);
  if (!proof) {
    return failedReport<rustaxa::HostVrfReport>(request.effect_id, "VRF_PROOF_FAILED");
  }
  const auto output = vrf_wrapper::getVrfOutput(wallet.vrf_pk, *proof, message);
  if (!output) {
    return failedReport<rustaxa::HostVrfReport>(request.effect_id, "VRF_OUTPUT_FAILED");
  }

  rustaxa::HostVrfReport report{};
  report.effect_id = request.effect_id;
  report.succeeded = true;
  report.proof = toRustBytes(proof->asBytes());
  report.output = toRustBytes(output->asBytes());
  return report;
}

class ConsensusTransportPort::Impl final {
 public:
  std::shared_ptr<Network> network() const {
    const std::scoped_lock lock(mutex);
    return network_.lock();
  }

  void attach(std::weak_ptr<Network> network) {
    const std::scoped_lock lock(mutex);
    network_ = std::move(network);
  }

 private:
  mutable std::mutex mutex;
  std::weak_ptr<Network> network_;
};

ConsensusTransportPort::ConsensusTransportPort() : impl_(std::make_unique<Impl>()) {}
ConsensusTransportPort::~ConsensusTransportPort() = default;

rustaxa::HostTransportReport ConsensusTransportPort::consensusGossipVote(
    const rustaxa::HostGossipVoteRequest& request) const {
  const auto network = impl_->network();
  if (!network) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "TRANSPORT_UNAVAILABLE");
  }
  try {
    auto vote = std::make_shared<PbftVote>(fromRustBytes(request.vote_rlp));
    std::shared_ptr<PbftBlock> block;
    if (!request.proposed_block_rlp.empty()) {
      block = std::make_shared<PbftBlock>(fromRustBytes(request.proposed_block_rlp));
    }
    network->gossipVote(vote, block, request.rebroadcast);
    return successfulTransportReport(request.effect_id);
  } catch (const std::exception&) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "GOSSIP_VOTE_FAILED");
  }
}

rustaxa::HostTransportReport ConsensusTransportPort::consensusGossipVoteBundle(
    const rustaxa::HostGossipVoteBundleRequest& request) const {
  const auto network = impl_->network();
  if (!network) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "TRANSPORT_UNAVAILABLE");
  }
  try {
    const auto votes_bundle_rlp = fromRustBytes(request.votes_bundle_rlp);
    auto votes = decodePbftVotesBundleRlp(dev::RLP(votes_bundle_rlp));
    network->gossipVotesBundle(votes, request.rebroadcast);
    return successfulTransportReport(request.effect_id);
  } catch (const std::exception&) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "GOSSIP_VOTE_BUNDLE_FAILED");
  }
}

rustaxa::HostTransportReport ConsensusTransportPort::consensusGossipPillarVote(
    const rustaxa::HostGossipPillarVoteRequest& request) const {
  const auto network = impl_->network();
  if (!network) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "TRANSPORT_UNAVAILABLE");
  }
  try {
    network->gossipPillarBlockVote(std::make_shared<PillarVote>(fromRustBytes(request.pillar_vote_rlp)),
                                   request.rebroadcast);
    return successfulTransportReport(request.effect_id);
  } catch (const std::exception&) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "GOSSIP_PILLAR_VOTE_FAILED");
  }
}

rustaxa::HostTransportStatus ConsensusTransportPort::consensusTransportStatus() const {
  rustaxa::HostTransportStatus status{};
  if (const auto network = impl_->network()) {
    status.available = true;
    status.packet_queue_over_limit = network->packetQueueOverLimit();
  }
  return status;
}

rustaxa::HostTransportReport ConsensusTransportPort::consensusReportMaliciousPeer(
    const rustaxa::HostMaliciousPeerRequest& request) const {
  const auto network = impl_->network();
  if (!network) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "TRANSPORT_UNAVAILABLE");
  }
  network->handleMaliciousSyncPeer(dev::p2p::NodeID(request.peer_id.data(), dev::p2p::NodeID::ConstructFromPointer));
  return successfulTransportReport(request.effect_id);
}

namespace {

class ConsensusVdfExecutor final {
 public:
  // The legacy VDF type overrides a virtual formatter but has no virtual
  // destructor. Keep the concrete asynchronous job allocation final so its
  // exact destructor is always selected without changing the upstream type.
  class VdfProofJob final : public vdf_sortition::VdfSortition {
   public:
    using vdf_sortition::VdfSortition::VdfSortition;
  };

  explicit ConsensusVdfExecutor(const FullNodeConfig& config) : wallets(config.wallets) {}

  ~ConsensusVdfExecutor() {
    std::vector<std::unique_ptr<Job>> pending;
    {
      const std::scoped_lock lock(mutex);
      pending.reserve(jobs.size());
      for (auto& [_, job] : jobs) {
        job->cancelled->store(true, std::memory_order_relaxed);
        pending.push_back(std::move(job));
      }
      jobs.clear();
    }
    for (auto& job : pending) {
      job->computation.wait();
    }
  }

  rustaxa::HostDagVdfStartReport start(const rustaxa::HostDagVdfRequest& request);
  rustaxa::HostDagVdfPollReport poll(const rustaxa::HostDagVdfJobRequest& request);
  rustaxa::HostDagVdfCancelReport cancel(const rustaxa::HostDagVdfJobRequest& request);

  struct Job {
    std::shared_ptr<VdfProofJob> proof;
    std::shared_ptr<std::atomic_bool> cancelled;
    std::future<void> computation;
  };

  uint64_t nextJobId() {
    auto id = next_job_id++;
    if (id == 0) {
      id = next_job_id++;
    }
    return id;
  }

  std::vector<WalletConfig> wallets;
  std::mutex mutex;
  uint64_t next_job_id{1};
  std::unordered_map<uint64_t, std::unique_ptr<Job>> jobs;
};

rustaxa::HostDagVdfStartReport ConsensusVdfExecutor::start(const rustaxa::HostDagVdfRequest& request) {
  rustaxa::HostDagVdfStartReport report{};
  report.effect_id = request.effect_id;
  if (request.wallet_index >= wallets.size()) {
    report.error_code = rust::String("VDF_WALLET_INDEX_OUT_OF_RANGE");
    return report;
  }
  if (request.max_vote_count == 0 || request.lambda_bound == 0) {
    report.error_code = rust::String("VDF_REQUEST_INVALID");
    return report;
  }

  try {
    const auto& wallet = wallets[request.wallet_index];
    const auto vrf_proof = vrf_wrapper::getVrfProof(wallet.vrf_secret, fromRustBytes(request.vrf_input));
    if (!vrf_proof) {
      report.error_code = rust::String("VDF_VRF_PROOF_FAILED");
      return report;
    }

    dev::RLPStream encoded;
    encoded.appendList(4) << *vrf_proof << dev::bytes{} << dev::bytes{} << request.difficulty;
    auto proof = std::make_shared<VdfProofJob>(encoded.invalidate());
    auto cancelled = std::make_shared<std::atomic_bool>(false);
    const SortitionParams execution_config{/*threshold_upper=*/1,
                                           /*min=*/request.difficulty,
                                           /*max=*/request.difficulty,
                                           /*stale=*/request.difficulty,
                                           /*lambda_max_bound=*/request.lambda_bound};
    auto job = std::make_unique<Job>();
    job->proof = proof;
    job->cancelled = cancelled;
    job->computation = std::async(std::launch::async,
                                  [proof, cancelled, execution_config, message = fromRustBytes(request.vdf_message)] {
                                    proof->computeVdfSolution(execution_config, message, *cancelled);
                                  });

    {
      const std::scoped_lock lock(mutex);
      report.job_id = nextJobId();
      jobs.emplace(report.job_id, std::move(job));
    }
    report.started = true;
  } catch (const std::exception& error) {
    report.error_code = rust::String(std::string("VDF_START_FAILED: ") + error.what());
  }
  return report;
}

rustaxa::HostDagVdfPollReport ConsensusVdfExecutor::poll(const rustaxa::HostDagVdfJobRequest& request) {
  rustaxa::HostDagVdfPollReport report{};
  report.effect_id = request.effect_id;
  report.job_id = request.job_id;
  std::unique_ptr<Job> job;
  {
    const std::scoped_lock lock(mutex);
    const auto found = jobs.find(request.job_id);
    if (found == jobs.end()) {
      report.error_code = rust::String("VDF_JOB_NOT_FOUND");
      return report;
    }
    if (found->second->computation.wait_for(std::chrono::milliseconds(0)) != std::future_status::ready) {
      return report;
    }
    job = std::move(found->second);
    jobs.erase(found);
  }

  report.complete = true;
  try {
    job->computation.get();
    report.cancelled = job->cancelled->load(std::memory_order_relaxed);
    if (report.cancelled) {
      report.error_code = rust::String("VDF_CANCELLED");
    } else {
      report.vdf_rlp = toRustBytes(job->proof->rlp());
      report.succeeded = true;
    }
  } catch (const std::exception& error) {
    report.cancelled = job->cancelled->load(std::memory_order_relaxed);
    report.error_code = rust::String(std::string("VDF_PROOF_FAILED: ") + error.what());
  }
  return report;
}

rustaxa::HostDagVdfCancelReport ConsensusVdfExecutor::cancel(const rustaxa::HostDagVdfJobRequest& request) {
  rustaxa::HostDagVdfCancelReport report{};
  report.effect_id = request.effect_id;
  report.job_id = request.job_id;
  std::unique_ptr<Job> job;
  {
    const std::scoped_lock lock(mutex);
    const auto found = jobs.find(request.job_id);
    if (found == jobs.end()) {
      report.error_code = rust::String("VDF_JOB_NOT_FOUND");
      return report;
    }
    found->second->cancelled->store(true, std::memory_order_relaxed);
    job = std::move(found->second);
    jobs.erase(found);
  }
  try {
    job->computation.get();
    report.cancelled = true;
  } catch (const std::exception& error) {
    report.cancelled = true;
    report.error_code = rust::String(std::string("VDF_CANCEL_FAILED: ") + error.what());
  }
  return report;
}

class ConsensusObserverExecutor final {
 public:
  explicit ConsensusObserverExecutor(std::shared_ptr<ConsensusApplication> application)
      : application(std::move(application)) {
    if (!this->application) {
      throw std::invalid_argument("ConsensusObserverExecutor requires ConsensusApplication");
    }
  }

  rustaxa::HostConsensusObservationReport observe(const rustaxa::HostConsensusObservationRequest& request);

  std::shared_ptr<ConsensusApplication> application;
};

rustaxa::HostConsensusObservationReport ConsensusObserverExecutor::observe(
    const rustaxa::HostConsensusObservationRequest& request) {
  rustaxa::HostConsensusObservationReport report{};
  report.effect_id = request.effect_id;
  try {
    const auto hash = h256(request.hash.data(), h256::ConstructFromPointer);
    if (request.kind == 1) {
      const Transaction transaction(fromRustBytes(request.canonical_rlp));
      if (transaction.getHash() != hash) {
        return failedReport<rustaxa::HostConsensusObservationReport>(request.effect_id,
                                                                     "OBSERVED_TRANSACTION_HASH_MISMATCH");
      }
      application->publishTransactionObserved(hash);
    } else if (request.kind == 2) {
      auto block = std::make_shared<DagBlock>(fromRustBytes(request.canonical_rlp));
      if (block->getHash() != hash) {
        return failedReport<rustaxa::HostConsensusObservationReport>(request.effect_id,
                                                                     "OBSERVED_DAG_BLOCK_HASH_MISMATCH");
      }
      application->publishDagBlockObserved(block);
    } else if (request.kind == 3) {
      const auto canonical_rlp = fromRustBytes(request.canonical_rlp);
      const dev::RLP decoded(canonical_rlp);
      const pillar_chain::PillarBlockData block_data(decoded);
      if (!block_data.block_ || block_data.block_->getHash() != hash) {
        return failedReport<rustaxa::HostConsensusObservationReport>(request.effect_id,
                                                                     "OBSERVED_PILLAR_BLOCK_HASH_MISMATCH");
      }
      application->publishPillarBlockObserved(block_data);
    } else if (request.kind == 4) {
      if (!request.canonical_rlp.empty() || request.period == 0) {
        return failedReport<rustaxa::HostConsensusObservationReport>(request.effect_id,
                                                                     "OBSERVED_FINALIZED_BLOCK_INVALID");
      }
      application->publishFinalizedBlockObserved(request.period, hash);
    } else {
      return failedReport<rustaxa::HostConsensusObservationReport>(request.effect_id, "OBSERVATION_KIND_UNSUPPORTED");
    }
    report.succeeded = true;
  } catch (const std::exception& error) {
    report.error_code = rust::String(std::string("OBSERVATION_FAILED: ") + error.what());
  }
  return report;
}

}  // namespace

ConsensusProcessPort::Impl::Impl(const FullNodeConfig& config, std::shared_ptr<ConsensusApplication> application)
    : vdf(std::make_unique<ConsensusVdfExecutor>(config)),
      observer(std::make_unique<ConsensusObserverExecutor>(std::move(application))) {}

ConsensusProcessPort::Impl::~Impl() = default;

ConsensusProcessPort::ConsensusProcessPort() : impl_(std::make_unique<Impl>()) {}

ConsensusProcessPort::ConsensusProcessPort(const FullNodeConfig& config,
                                           std::shared_ptr<ConsensusApplication> application)
    : impl_(std::make_unique<Impl>(config, std::move(application))) {}

ConsensusProcessPort::~ConsensusProcessPort() = default;

rustaxa::HostDagVdfStartReport ConsensusProcessPort::consensusStartDagVdf(
    const rustaxa::HostDagVdfRequest& request) const {
  if (!impl_->vdf) {
    rustaxa::HostDagVdfStartReport report{};
    report.effect_id = request.effect_id;
    report.error_code = rust::String("VDF_EXECUTOR_UNAVAILABLE");
    return report;
  }
  return impl_->vdf->start(request);
}

rustaxa::HostDagVdfPollReport ConsensusProcessPort::consensusPollDagVdf(
    const rustaxa::HostDagVdfJobRequest& request) const {
  if (!impl_->vdf) {
    rustaxa::HostDagVdfPollReport report{};
    report.effect_id = request.effect_id;
    report.job_id = request.job_id;
    report.error_code = rust::String("VDF_EXECUTOR_UNAVAILABLE");
    return report;
  }
  return impl_->vdf->poll(request);
}

rustaxa::HostDagVdfCancelReport ConsensusProcessPort::consensusCancelDagVdf(
    const rustaxa::HostDagVdfJobRequest& request) const {
  if (!impl_->vdf) {
    rustaxa::HostDagVdfCancelReport report{};
    report.effect_id = request.effect_id;
    report.job_id = request.job_id;
    report.error_code = rust::String("VDF_EXECUTOR_UNAVAILABLE");
    return report;
  }
  return impl_->vdf->cancel(request);
}

rustaxa::HostConsensusObservationReport ConsensusProcessPort::consensusObserve(
    const rustaxa::HostConsensusObservationRequest& request) const {
  if (!impl_->observer) {
    return failedReport<rustaxa::HostConsensusObservationReport>(request.effect_id, "OBSERVER_EXECUTOR_UNAVAILABLE");
  }
  return impl_->observer->observe(request);
}

class ExternalEvmPort::Impl final {
 public:
  explicit Impl(std::shared_ptr<final_chain::FinalChain> final_chain)
      : owner(std::move(final_chain)), final_chain(owner.get()) {
    if (!this->final_chain) {
      throw std::invalid_argument("ExternalEvmPort requires FinalChain");
    }
  }
  explicit Impl(final_chain::FinalChain& final_chain) : final_chain(&final_chain) {}

  std::shared_ptr<final_chain::FinalChain> owner;
  final_chain::FinalChain* final_chain;
};

ExternalEvmPort::ExternalEvmPort(std::shared_ptr<final_chain::FinalChain> final_chain)
    : impl_(std::make_unique<Impl>(std::move(final_chain))) {}
ExternalEvmPort::ExternalEvmPort(final_chain::FinalChain& final_chain) : impl_(std::make_unique<Impl>(final_chain)) {}
ExternalEvmPort::~ExternalEvmPort() = default;

rustaxa::HostPillarAnchorStateReport ExternalEvmPort::consensusLoadPillarAnchorState(
    const rustaxa::HostPillarAnchorStateRequest& request) const {
  rustaxa::HostPillarAnchorStateReport report{};
  report.effect_id = request.effect_id;
  try {
    const auto header = impl_->final_chain->blockHeader(request.period);
    if (!header) {
      return failedReport<rustaxa::HostPillarAnchorStateReport>(request.effect_id, "PILLAR_ANCHOR_HEADER_MISSING");
    }
    report.succeeded = true;
    report.block_header_rlp = toRustBytes(util::rlp_enc(*header));
    report.state_root = header->state_root.asArray();
    report.bridge_root = impl_->final_chain->getBridgeRoot(request.period).asArray();
    report.bridge_epoch = impl_->final_chain->getBridgeEpoch(request.period).asArray();
    const auto validator_vote_counts =
        impl_->final_chain->dposValidatorsEligibleVoteCounts(request.pillar_block_period);
    report.validator_vote_counts.reserve(validator_vote_counts.size());
    for (const auto& validator : validator_vote_counts) {
      rustaxa::HostValidatorVoteCount fact{};
      fact.address = validator.addr.asArray();
      fact.vote_count = validator.vote_count;
      report.validator_vote_counts.push_back(std::move(fact));
    }
    report.signer_vote_counts.reserve(request.signer_addresses.size());
    for (const auto& signer : request.signer_addresses) {
      report.signer_vote_counts.push_back(
          impl_->final_chain->dposEligibleVoteCount(request.pillar_block_period, fromHostAddress(signer.bytes)));
    }
    report.total_eligible_vote_count = impl_->final_chain->dposEligibleTotalVoteCount(request.pillar_block_period);
  } catch (const std::exception& error) {
    const auto error_code = std::string("PILLAR_ANCHOR_STATE_READ_FAILED: ") + error.what();
    return failedReport<rustaxa::HostPillarAnchorStateReport>(request.effect_id, error_code.c_str());
  }
  return report;
}

rustaxa::HostFinalChainAccountFactsReport ExternalEvmPort::consensusLoadFinalChainAccountFacts(
    const rustaxa::HostFinalChainAccountFactsRequest& request) const {
  rustaxa::HostFinalChainAccountFactsReport report{};
  report.effect_id = request.effect_id;
  try {
    report.observed_block = impl_->final_chain->lastBlockNumber();
    report.accounts.reserve(request.addresses.size());
    for (const auto& requested : request.addresses) {
      const auto address = fromHostAddress(requested.bytes);
      const auto account = impl_->final_chain->getAccount(address, report.observed_block);
      rustaxa::HostFinalChainAccountFact fact{};
      fact.address = requested.bytes;
      fact.found = account.has_value();
      if (account) {
        fact.nonce = toBridgeU256(account->nonce);
        fact.balance = toBridgeU256(account->balance);
      }
      report.accounts.push_back(std::move(fact));
    }
    report.succeeded = true;
  } catch (const std::exception&) {
    return failedReport<rustaxa::HostFinalChainAccountFactsReport>(request.effect_id,
                                                                   "FINAL_CHAIN_ACCOUNT_FACTS_READ_FAILED");
  }
  return report;
}

rustaxa::HostDagGasBatch ExternalEvmPort::consensusEstimateDagTransactionGas(
    const rustaxa::HostDagGasBatch& request) const {
  rustaxa::HostDagGasBatch report{};
  report.effect_id = request.effect_id;
  report.proposal_period = request.proposal_period;
  report.transaction_hashes.reserve(request.transaction_hashes.size());
  for (const auto& hash : request.transaction_hashes) {
    report.transaction_hashes.push_back(hash);
  }
  try {
    if (request.transaction_hashes.size() != request.transaction_rlps.size()) {
      report.error_code = rust::String("DAG_GAS_TRANSACTION_COUNT_MISMATCH");
      return report;
    }
    report.observed_block = impl_->final_chain->lastBlockNumber();
    report.gas_used.reserve(request.transaction_hashes.size());
    report.result_rlps.reserve(request.transaction_hashes.size());
    for (size_t index = 0; index < request.transaction_hashes.size(); ++index) {
      const auto transaction = std::make_shared<Transaction>(fromRustBytes(request.transaction_rlps[index].data));
      if (transaction->getHash().asArray() != request.transaction_hashes[index].hash) {
        report.gas_used.clear();
        report.result_rlps.clear();
        report.error_code = rust::String("DAG_GAS_TRANSACTION_HASH_MISMATCH");
        return report;
      }
      const auto evm_transaction = state_api::EVMTransaction{
          transaction->getSender(), transaction->getGasPrice(), transaction->getReceiver(), transaction->getNonce(),
          transaction->getValue(),  transaction->getGas(),      transaction->getData(),
      };
      const auto result = impl_->final_chain->call(evm_transaction, request.proposal_period);
      report.gas_used.push_back(result.gas_used);
      rustaxa::CanonicalBytes result_rlp{};
      result_rlp.data = toRustBytes(util::rlp_enc(result));
      report.result_rlps.push_back(std::move(result_rlp));
    }
    report.succeeded = true;
  } catch (const std::exception& error) {
    report.gas_used.clear();
    report.result_rlps.clear();
    report.error_code = rust::String(std::string("DAG_GAS_ESTIMATION_FAILED: ") + error.what());
  }
  return report;
}

rustaxa::HostFinalChainSystemFactsReport ExternalEvmPort::consensusLoadFinalChainSystemFacts(
    const rustaxa::HostFinalChainSystemFactsRequest& request) const {
  try {
    return impl_->final_chain->external_evm_state_api_.loadSystemTransactionFacts(request);
  } catch (const std::exception& error) {
    rustaxa::HostFinalChainSystemFactsReport report{};
    report.request_id = request.request_id;
    report.period = request.period;
    report.error_code = rust::String(std::string("FINAL_CHAIN_SYSTEM_FACTS_FAILED: ") + error.what());
    return report;
  }
}

rustaxa::HostFinalChainPreflightReport ExternalEvmPort::consensusLoadFinalChainCommittedState(
    const rustaxa::HostFinalChainPreflightRequest& request) const {
  try {
    return impl_->final_chain->external_evm_state_api_.loadCommittedState(request);
  } catch (const std::exception& error) {
    rustaxa::HostFinalChainPreflightReport report{};
    report.request_id = request.request_id;
    report.error_code = rust::String(std::string("FINAL_CHAIN_PREFLIGHT_FAILED: ") + error.what());
    return report;
  }
}

rustaxa::HostFinalChainExecutionReport ExternalEvmPort::consensusExecuteFinalChainTransactions(
    const rustaxa::HostFinalChainExecutionRequest& request) const {
  try {
    return impl_->final_chain->external_evm_state_api_.executeTransactions(request);
  } catch (const std::exception& error) {
    rustaxa::HostFinalChainExecutionReport report{};
    report.request_id = request.request_id;
    report.status = 1;
    report.error_code = rust::String(std::string("FINAL_CHAIN_EXECUTION_FAILED: ") + error.what());
    return report;
  }
}

rustaxa::HostFinalChainRewardsReport ExternalEvmPort::consensusDistributeFinalChainRewards(
    const rustaxa::HostFinalChainRewardsRequest& request) const {
  try {
    return impl_->final_chain->external_evm_state_api_.distributeRewards(request);
  } catch (const std::exception& error) {
    rustaxa::HostFinalChainRewardsReport report{};
    report.request_id = request.request_id;
    report.period = request.period;
    report.status = 1;
    report.error_code = rust::String(std::string("FINAL_CHAIN_REWARDS_FAILED: ") + error.what());
    return report;
  }
}

rustaxa::HostFinalChainStateCommitReport ExternalEvmPort::consensusCommitFinalChainState(
    const rustaxa::HostFinalChainStateCommitRequest& request) const {
  return impl_->final_chain->external_evm_state_api_.commitState(request);
}

class ConsensusProcess::Impl final {
 public:
  enum class LifecycleState : uint8_t { kStopped, kRunning };

  Impl(std::shared_ptr<ConsensusApplication> application, const FullNodeConfig& config,
       std::shared_ptr<final_chain::FinalChain> final_chain)
      : application(std::move(application)),
        process(config, this->application),
        signer(config),
        evm(std::move(final_chain)) {
    if (!this->application) {
      throw std::invalid_argument("ConsensusProcess requires ConsensusApplication");
    }
  }

  std::shared_ptr<ConsensusApplication> application;
  ConsensusProcessPort process;
  ConsensusSignerPort signer;
  ConsensusTransportPort transport;
  ExternalEvmPort evm;
  // Serializes complete public start/stop transitions, including joins. The
  // worker never acquires this mutex, so stop may retain it while joining.
  std::mutex lifecycle_mutex;
  // Protects only the worker handle and state; never held while joining or
  // while the native runner blocks.
  std::mutex state_mutex;
  std::thread worker;
  LifecycleState state{LifecycleState::kStopped};
};

ConsensusProcess::ConsensusProcess(std::shared_ptr<ConsensusApplication> application, const FullNodeConfig& config,
                                   std::shared_ptr<final_chain::FinalChain> final_chain)
    : impl_(std::make_unique<Impl>(std::move(application), config, std::move(final_chain))) {}

ConsensusProcess::~ConsensusProcess() { stop(); }

void ConsensusProcess::attachNetwork(std::weak_ptr<Network> network) {
  impl_->transport.impl_->attach(std::move(network));
}

void ConsensusProcess::start() {
  const std::scoped_lock lifecycle_lock(impl_->lifecycle_mutex);
  std::thread finished_worker;
  {
    const std::scoped_lock state_lock(impl_->state_mutex);
    if (impl_->state == Impl::LifecycleState::kRunning) {
      return;
    }
    if (impl_->worker.joinable()) {
      finished_worker = std::move(impl_->worker);
    }
  }
  if (finished_worker.joinable()) {
    finished_worker.join();
  }

  const std::scoped_lock state_lock(impl_->state_mutex);
  impl_->process.impl_->reset();
  impl_->state = Impl::LifecycleState::kRunning;
  try {
    impl_->worker = std::thread([impl = impl_.get()] {
      try {
        static_cast<void>(rustaxa::consensus_application_run(impl->application->service(), impl->process, impl->signer,
                                                             impl->transport, impl->evm));
      } catch (const std::exception& error) {
        std::fprintf(stderr, "native consensus process terminated: %s\n", error.what());
      } catch (...) {
        std::fprintf(stderr, "native consensus process terminated with an unknown error\n");
      }
      const std::scoped_lock state_lock(impl->state_mutex);
      impl->state = Impl::LifecycleState::kStopped;
    });
  } catch (...) {
    impl_->state = Impl::LifecycleState::kStopped;
    throw;
  }
}

void ConsensusProcess::stop() noexcept {
  const std::scoped_lock lifecycle_lock(impl_->lifecycle_mutex);
  std::thread worker;
  {
    const std::scoped_lock state_lock(impl_->state_mutex);
    if (impl_->state == Impl::LifecycleState::kStopped && !impl_->worker.joinable()) {
      return;
    }
    impl_->process.impl_->stop();
    worker = std::move(impl_->worker);
    impl_->state = Impl::LifecycleState::kStopped;
  }
  if (worker.joinable()) {
    worker.join();
  }
}

}  // namespace taraxa

#endif  // RUSTAXA_ENABLE
