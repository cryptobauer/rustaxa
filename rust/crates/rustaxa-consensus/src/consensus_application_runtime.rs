//! Native blocking lifecycle and host-leaf boundary for PBFT consensus.
//!
//! This module owns restart generations, effect identities, daemon scheduling,
//! and validation of host reports. Host ports expose only OS process mechanics,
//! key custody, physical transport, and concrete external-EVM execution. They
//! never receive a manager action or mutable consensus object.

use crate::FinalChain;
use crate::consensus_application::DagProposerConfig;
use crate::consensus_application_startup::{
    apply_startup_persisted_pillar_vote, apply_startup_pillar_anchor_state,
    apply_startup_pillar_vote, complete_consensus_startup, hydrate_recently_finalized_transactions,
    prepare_consensus_startup,
};
use crate::consensus_state_actions::{
    ConsensusStateActionRequest, ConsensusStateVoteCommit, ConsensusStateVoteTask,
    compose_consensus_state_action,
};
use crate::consensus_value_proposal::{
    ConsensusValueProposalAction, complete_value_proposal_signing, compose_value_proposal,
};
use crate::dag_service::{
    DagProposerAddBlockReport, DagProposerSessionAction, DagProposerSessionBeginInput,
    DagProposerSigningReport, DagProposerVdfProofReport, DagProposerVrfReport,
};
use crate::dag_transaction_service::{
    DagAddBlockAccountNonceFact, DagAddBlockCompletion, DagAddBlockPrepareRequest,
    DagAddBlockTransactionPayload, DagProposerPackPrepareRequest, DagTransactionService,
};
use crate::maybe_broadcast_votes::{
    ConsensusVoteTransportRequest, MaybeBroadcastVotesActionId, MaybeBroadcastVotesBatch,
    MaybeBroadcastVotesCommit, MaybeBroadcastVotesInput, VoteBroadcastAcknowledgement,
    VoteBroadcastCounters, select_maybe_broadcast_votes,
    validate_maybe_broadcast_votes_acknowledgements,
};
use crate::pbft_application_finalization::{
    PbftApplicationAccountFact, PbftApplicationAccountFactsReport, PbftApplicationEvmReport,
    PbftApplicationFinalizationRequest, PbftApplicationFinalizationStep,
    PbftApplicationPillarAnchorReport, PbftApplicationPillarObservation,
    prepare_certified_pbft_application_finalization, prepare_pbft_application_finalization,
    report_pbft_application_finalization_account_facts, report_pbft_application_finalization_evm,
    report_pbft_application_finalization_pillar_anchor,
    report_pbft_application_finalization_pillar_gossip,
    report_pbft_application_finalization_pillar_signature,
};
use crate::pbft_manager::{
    PbftManagerRuntimeAction, PbftManagerRuntimeActionReport, PbftManagerRuntimeActionResultCode,
    PbftManagerRuntimeStatus, PbftManagerRuntimeTickFact,
};
use crate::pbft_service::{
    PbftLifecycleActionStatus, PbftLocalProposalCandidate, PbftLocalProposalSelectionRequest,
    PbftProcessSyncedLeaves, PbftRoundAdvanceActionOutcome, PbftService,
};
use crate::pbft_vote_generation::{
    PbftGeneratedVote, PbftVoteGenerationPublicInput, complete_pbft_vote_signing,
    prepare_pbft_vote_vrf,
};
use crate::pbft_vote_validation::{PbftPublicProposerSortitionInput, prepare_public_proposer_vrf};
use crate::transaction_packing_service::TransactionPackingEstimate;
use crate::vdf_executor::{NativeVdfExecutor, NativeVdfPollResult, NativeVdfRequest};
use crate::verified_votes::PbftVoteType;
use crate::verified_votes::TwoTPlusOneVotedBlockType;
use anyhow::{Context, Result, bail, ensure};
use rlp::Rlp;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const CONSENSUS_OBSERVATION_KIND_PILLAR_BLOCK: u8 = 3;
const CONSENSUS_OBSERVATION_KIND_FINALIZED_BLOCK: u8 = 4;

/// Stable identity of one host effect within a restartable runtime generation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConsensusEffectId {
    /// Monotonic native run generation. Zero is never issued.
    pub generation: u64,
    /// Monotonic effect sequence within `generation`. Zero is never issued.
    pub sequence: u64,
}

/// Public identity for host-held signing material.
///
/// The stable wallet index selects the host key. Secret bytes never enter the
/// native application configuration or any request/report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SigningIdentity {
    pub wallet_index: u64,
    pub address: [u8; 20],
    pub node_public_key: [u8; 64],
    pub vrf_public_key: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusWaitRequest {
    pub effect_id: ConsensusEffectId,
    pub delay_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsensusWaitOutcome {
    Elapsed,
    Stopped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusWaitReport {
    pub effect_id: ConsensusEffectId,
    pub outcome: ConsensusWaitOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ConsensusTimingOrigins {
    period: u64,
    round: u64,
    period_started_ms: u64,
    round_started_ms: u64,
}

impl ConsensusTimingOrigins {
    fn new(now_ms: u64) -> Self {
        Self {
            period: 0,
            round: 0,
            period_started_ms: now_ms,
            round_started_ms: now_ms,
        }
    }

    /// Observes the authoritative cursor and resets both timing epochs when a
    /// period advances, even when the new period starts at the same round.
    fn observe(&mut self, period: u64, round: u64, now_ms: u64) {
        if period != self.period {
            self.period = period;
            self.round = round;
            self.period_started_ms = now_ms;
            self.round_started_ms = now_ms;
        } else if round != self.round {
            self.round = round;
            self.round_started_ms = now_ms;
        }
    }

    fn period_elapsed_ms(self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.period_started_ms)
    }

    fn round_elapsed_ms(self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.round_started_ms)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusSignRequest {
    pub effect_id: ConsensusEffectId,
    pub wallet_index: u64,
    pub digest: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusSignReport {
    pub effect_id: ConsensusEffectId,
    pub succeeded: bool,
    pub signature: Vec<u8>,
    pub error_code: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusVrfRequest {
    pub effect_id: ConsensusEffectId,
    pub wallet_index: u64,
    pub message: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusVrfReport {
    pub effect_id: ConsensusEffectId,
    pub succeeded: bool,
    pub proof: Vec<u8>,
    pub output: Vec<u8>,
    pub error_code: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GossipVoteRequest {
    pub effect_id: ConsensusEffectId,
    pub vote_rlp: Vec<u8>,
    pub proposed_block_rlp: Vec<u8>,
    pub rebroadcast: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GossipVoteBundleRequest {
    pub effect_id: ConsensusEffectId,
    pub votes_bundle_rlp: Vec<u8>,
    pub rebroadcast: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GossipPillarVoteRequest {
    pub effect_id: ConsensusEffectId,
    pub pillar_vote_rlp: Vec<u8>,
    pub rebroadcast: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportMaliciousPeerRequest {
    pub effect_id: ConsensusEffectId,
    pub peer_id: [u8; 64],
    pub evidence_rlp: Vec<u8>,
}

/// Exact canonical DAG-block transport leaf.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GossipDagBlockRequest {
    pub effect_id: ConsensusEffectId,
    pub block_hash: [u8; 32],
    pub block_rlp: Vec<u8>,
}

/// One canonical transaction requiring concrete EVM gas estimation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DagGasEstimateInput {
    pub hash: [u8; 32],
    pub transaction_rlp: Vec<u8>,
}

/// Exact unlocked DAG proposer gas-estimation leaf.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DagGasEstimateRequest {
    pub effect_id: ConsensusEffectId,
    pub proposal_period: u64,
    pub transactions: Vec<DagGasEstimateInput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DagGasEstimateResult {
    pub hash: [u8; 32],
    pub gas_used: u64,
    pub result_rlp: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DagGasEstimateReport {
    pub effect_id: ConsensusEffectId,
    pub succeeded: bool,
    pub observed_block: u64,
    pub estimates: Vec<DagGasEstimateResult>,
    pub error_code: String,
}

/// Public observer fact emitted after native DAG/transaction publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusObservationRequest {
    pub effect_id: ConsensusEffectId,
    pub kind: u8,
    /// Finalized period for block observations; zero for other event kinds.
    pub period: u64,
    pub hash: [u8; 32],
    pub canonical_rlp: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusObservationReport {
    pub effect_id: ConsensusEffectId,
    pub succeeded: bool,
    pub error_code: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusTransportReport {
    pub effect_id: ConsensusEffectId,
    pub succeeded: bool,
    pub error_code: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConsensusTransportStatus {
    pub available: bool,
    /// Whether the host packet queue is applying proposal backpressure.
    pub packet_queue_over_limit: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvmFinalizationRequest {
    pub effect_id: ConsensusEffectId,
    pub period_data_rlp: Vec<u8>,
    pub previous_cert_vote_rlps: Vec<Vec<u8>>,
    /// Optional host-supplied legacy vote weights. Native callers leave this
    /// empty and use the weight embedded in canonical vote RLP.
    pub previous_cert_vote_weights: Vec<u64>,
    pub finalized_dag_hashes: Vec<[u8; 32]>,
    pub blocks_per_year: u32,
    pub synchronous: bool,
    pub anchor_block_rlp: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvmFinalizationReport {
    pub effect_id: ConsensusEffectId,
    pub succeeded: bool,
    pub status: u8,
    pub last_block_number: u64,
    pub error_code: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NativeFinalChainAccountFact {
    address: [u8; 20],
    found: bool,
    nonce: Vec<u8>,
    balance: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NativeFinalChainAccountFacts {
    observed_block: u64,
    accounts: Vec<NativeFinalChainAccountFact>,
}

/// Exact FinalChain/EVM read needed to reconstruct a due pillar block after restart.
///
/// The period is the already-finalized DPoS snapshot selected by native startup
/// policy. No general state handle or query callback crosses the boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PillarAnchorStateRequest {
    pub effect_id: ConsensusEffectId,
    /// Finalized state/header period selected after applying delegation delay.
    pub period: u64,
    /// Pillar block period whose DPoS validator snapshot must be returned.
    pub pillar_block_period: u64,
    /// Local signer addresses whose eligibility must be returned in order.
    pub signer_addresses: Vec<[u8; 20]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PillarAnchorValidatorVoteCount {
    pub address: [u8; 20],
    pub vote_count: u64,
}

/// Canonical finalized state returned for one startup pillar anchor read.
///
/// `block_header_rlp` is retained for boundary diagnostics and parity; native
/// pillar construction consumes the typed state root, bridge root, and bridge
/// epoch. Failed reports must leave all payload fields empty or zeroed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PillarAnchorStateReport {
    pub effect_id: ConsensusEffectId,
    pub succeeded: bool,
    pub block_header_rlp: Vec<u8>,
    pub state_root: [u8; 32],
    pub bridge_root: [u8; 32],
    pub bridge_epoch: [u8; 32],
    /// Complete nonzero validator vote-count snapshot in canonical host order.
    pub validator_vote_counts: Vec<PillarAnchorValidatorVoteCount>,
    /// Vote counts for `request.signer_addresses`, in the same order.
    pub signer_vote_counts: Vec<u64>,
    pub total_eligible_vote_count: u64,
    pub error_code: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupPersistedPillarVoteFacts {
    Retry,
    Ready {
        validator_vote_count: u64,
        total_eligible_vote_count: u64,
    },
}

fn startup_persisted_pillar_vote_facts(
    report: &PillarAnchorStateReport,
) -> Result<StartupPersistedPillarVoteFacts> {
    if !report.succeeded {
        return Ok(StartupPersistedPillarVoteFacts::Retry);
    }
    ensure!(
        report.signer_vote_counts.len() == 1,
        "CONSENSUS_STARTUP_PERSISTED_PILLAR_VOTE_FACT_COUNT_MISMATCH"
    );
    Ok(StartupPersistedPillarVoteFacts::Ready {
        validator_vote_count: report.signer_vote_counts[0],
        total_eligible_vote_count: report.total_eligible_vote_count,
    })
}

/// Classifies one persisted-vote anchor attempt without weakening effect
/// correlation. Concrete-EVM or native-state unavailability is retryable
/// during startup, while a stale effect report is an application invariant
/// violation and must fail the run instead of entering an infinite retry.
fn startup_persisted_pillar_vote_report(
    report: Result<PillarAnchorStateReport>,
) -> Result<StartupPersistedPillarVoteFacts> {
    match report {
        Ok(report) => startup_persisted_pillar_vote_facts(&report),
        Err(error)
            if error.chain().any(|cause| {
                matches!(
                    cause.to_string().as_str(),
                    "CONSENSUS_RUNTIME_STALE_EFFECT_REPORT"
                        | "CONSENSUS_RUNTIME_STALE_EFFECT_GENERATION"
                )
            }) =>
        {
            Err(error)
        }
        Err(_) => Ok(StartupPersistedPillarVoteFacts::Retry),
    }
}

/// Interruptible process/timer mechanics used by the blocking runner.
pub trait ConsensusProcessPort {
    /// Monotonic clock used only for scheduling and elapsed-time decisions.
    fn now_millis(&self) -> u64;
    /// Unix wall-clock seconds used for canonical PBFT block timestamps.
    fn unix_time_seconds(&self) -> u64;
    fn wait(&self, request: &ConsensusWaitRequest) -> Result<ConsensusWaitReport>;
    fn stop_requested(&self, generation: u64) -> bool;
}

/// Exact key-custody operations. Implementations retain all private keys.
pub trait ConsensusSigningPort {
    fn sign_digest(&self, request: &ConsensusSignRequest) -> Result<ConsensusSignReport>;
    fn prove_vrf(&self, request: &ConsensusVrfRequest) -> Result<ConsensusVrfReport>;
}

/// Named physical transport leaves over canonical payload bytes.
pub trait ConsensusTransportPort {
    fn gossip_vote(&self, request: &GossipVoteRequest) -> Result<ConsensusTransportReport>;
    fn gossip_vote_bundle(
        &self,
        request: &GossipVoteBundleRequest,
    ) -> Result<ConsensusTransportReport>;
    fn gossip_pillar_vote(
        &self,
        request: &GossipPillarVoteRequest,
    ) -> Result<ConsensusTransportReport>;
    fn transport_status(&self) -> ConsensusTransportStatus;
    fn report_malicious_peer(
        &self,
        request: &ReportMaliciousPeerRequest,
    ) -> Result<ConsensusTransportReport>;

    /// Gossips one already-published native DAG block.
    fn gossip_dag_block(
        &self,
        _request: &GossipDagBlockRequest,
    ) -> Result<ConsensusTransportReport> {
        bail!("CONSENSUS_DAG_GOSSIP_PORT_UNAVAILABLE")
    }
}

/// Public event/WebSocket publication leaf.
pub trait ConsensusObserverPort {
    fn observe(&self, request: &ConsensusObservationRequest) -> Result<ConsensusObservationReport>;
}

/// Concrete external-EVM leaves; sequencing and publication remain native.
pub trait ConsensusExecutionPort {
    fn load_final_chain_committed_state(
        &self,
        _request: &crate::FinalChainExternalEvmPreflightRequest,
    ) -> Result<crate::FinalChainExternalEvmPreflightReport> {
        bail!("CONSENSUS_FINAL_CHAIN_PREFLIGHT_PORT_UNAVAILABLE")
    }

    fn load_system_transaction_facts(
        &self,
        _request: &crate::FinalChainSystemTransactionFactsRequest,
    ) -> Result<crate::FinalChainSystemTransactionPlanFact> {
        bail!("CONSENSUS_SYSTEM_TRANSACTION_FACTS_PORT_UNAVAILABLE")
    }

    fn execute_final_chain_transactions(
        &self,
        _request: &crate::FinalChainEvmExecutionRequest,
    ) -> Result<crate::FinalChainEvmExecutionReport> {
        bail!("CONSENSUS_FINAL_CHAIN_EXECUTION_PORT_UNAVAILABLE")
    }

    fn distribute_final_chain_rewards(
        &self,
        _request: &crate::FinalChainEvmRewardsRequest,
    ) -> Result<crate::FinalChainEvmRewardsReport> {
        bail!("CONSENSUS_FINAL_CHAIN_REWARDS_PORT_UNAVAILABLE")
    }

    fn commit_final_chain_state(
        &self,
        _request: &crate::FinalChainExternalEvmStateCommitIntent,
    ) -> Result<crate::FinalChainExternalEvmStateCommitResult> {
        bail!("CONSENSUS_FINAL_CHAIN_STATE_COMMIT_PORT_UNAVAILABLE")
    }

    fn discard_final_chain_state(
        &self,
        _request: &crate::FinalChainExternalEvmDiscardRequest,
    ) -> Result<crate::FinalChainExternalEvmDiscardReport> {
        bail!("CONSENSUS_FINAL_CHAIN_STATE_DISCARD_PORT_UNAVAILABLE")
    }

    /// Loads the exact finalized state needed for restart pillar construction.
    fn load_pillar_anchor_state(
        &self,
        request: &PillarAnchorStateRequest,
    ) -> Result<PillarAnchorStateReport>;

    /// Estimates exact canonical transaction payloads for one proposer cursor.
    fn estimate_dag_transaction_gas(
        &self,
        _request: &DagGasEstimateRequest,
    ) -> Result<DagGasEstimateReport> {
        bail!("CONSENSUS_DAG_GAS_PORT_UNAVAILABLE")
    }
}

impl<T: ConsensusExecutionPort> crate::FinalChainExecutionLeaf for T {
    fn load_committed_state_descriptor(
        &self,
        request: &crate::FinalChainExternalEvmPreflightRequest,
    ) -> Result<crate::FinalChainExternalEvmPreflightReport> {
        self.load_final_chain_committed_state(request)
    }

    fn load_system_transaction_facts(
        &self,
        request: &crate::FinalChainSystemTransactionFactsRequest,
    ) -> Result<crate::FinalChainSystemTransactionPlanFact> {
        ConsensusExecutionPort::load_system_transaction_facts(self, request)
    }

    fn execute_transactions(
        &self,
        request: &crate::FinalChainEvmExecutionRequest,
    ) -> Result<crate::FinalChainEvmExecutionReport> {
        self.execute_final_chain_transactions(request)
    }

    fn distribute_rewards(
        &self,
        request: &crate::FinalChainEvmRewardsRequest,
    ) -> Result<crate::FinalChainEvmRewardsReport> {
        self.distribute_final_chain_rewards(request)
    }

    fn commit_staged_state(
        &self,
        request: &crate::FinalChainExternalEvmStateCommitIntent,
    ) -> Result<crate::FinalChainExternalEvmStateCommitResult> {
        self.commit_final_chain_state(request)
    }

    fn discard_staged_state(
        &self,
        request: &crate::FinalChainExternalEvmDiscardRequest,
    ) -> Result<crate::FinalChainExternalEvmDiscardReport> {
        self.discard_final_chain_state(request)
    }
}

struct RuntimeSyncedLeaves<'a, P, S, T> {
    runtime: &'a ConsensusApplicationRuntime,
    generation: u64,
    pbft: &'a PbftService,
    final_chain: &'a FinalChain,
    process: &'a P,
    signer: &'a S,
    transport: &'a T,
}

impl<P, S, T> PbftProcessSyncedLeaves for RuntimeSyncedLeaves<'_, P, S, T>
where
    P: ConsensusProcessPort,
    S: ConsensusSigningPort,
    T: ConsensusTransportPort,
{
    fn wait_for_finalization(&self) -> Result<()> {
        while !self.pbft.finalization_ready(self.final_chain)? {
            ensure!(
                self.runtime.wait_for(
                    self.generation,
                    self.runtime.polling_interval_ms,
                    self.process,
                )? != ConsensusWaitOutcome::Stopped,
                "CONSENSUS_RUNTIME_STOPPED_DURING_FINALIZATION_WAIT"
            );
        }
        Ok(())
    }

    fn report_malicious_peer(&self, peer_id: [u8; 64]) -> Result<()> {
        let id = self.runtime.next_effect(self.generation)?;
        let report = self
            .transport
            .report_malicious_peer(&ReportMaliciousPeerRequest {
                effect_id: id,
                peer_id,
                evidence_rlp: Vec::new(),
            })?;
        self.runtime.validate_report(id, report.effect_id)?;
        ensure!(
            report.succeeded,
            "CONSENSUS_RUNTIME_MALICIOUS_PEER_REPORT_FAILED: {}",
            report.error_code
        );
        Ok(())
    }

    fn sign_digest(&self, wallet_index: usize, digest: [u8; 32]) -> Result<Vec<u8>> {
        self.runtime
            .sign_digest(self.generation, wallet_index as u64, digest, self.signer)
    }
}

/// Terminal outcome from one blocking native run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsensusRunReason {
    Stopped,
    Completed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusRunExit {
    pub generation: u64,
    pub reason: ConsensusRunReason,
}

/// Restartable native PBFT daemon lifecycle and host-effect identity owner.
pub struct ConsensusApplicationRuntime {
    generation: AtomicU64,
    sequence: AtomicU64,
    operation_sequence: AtomicU64,
    running: AtomicBool,
    signing_identities: Vec<SigningIdentity>,
    polling_interval_ms: u64,
    dag_proposers: Vec<DagProposerSessionBeginInput>,
    dag_proposer_config: DagProposerConfig,
    bridge_contract_address: [u8; 20],
    max_levels_per_period: u64,
    vdf_executor: NativeVdfExecutor,
}

impl ConsensusApplicationRuntime {
    /// Constructs a runtime without DAG proposers, used by focused PBFT tests.
    pub fn new(signing_identities: Vec<SigningIdentity>, polling_interval_ms: u64) -> Result<Self> {
        Self::new_with_proposers(
            signing_identities,
            polling_interval_ms,
            Vec::new(),
            DagProposerConfig {
                total_transaction_shards: 1,
                proposal_dag_gas_limit: u64::MAX,
                default_dag_gas_limit: u64::MAX,
                default_pbft_gas_limit: u64::MAX,
                cornus_activation_period: u64::MAX,
                cornus_dag_gas_limit: u64::MAX,
                cornus_pbft_gas_limit: u64::MAX,
            },
        )
    }

    /// Constructs the complete runtime including key-custody-free DAG schedulers.
    pub fn new_with_proposers(
        signing_identities: Vec<SigningIdentity>,
        polling_interval_ms: u64,
        dag_proposers: Vec<DagProposerSessionBeginInput>,
        dag_proposer_config: DagProposerConfig,
    ) -> Result<Self> {
        Self::new_with_proposers_and_execution(
            signing_identities,
            polling_interval_ms,
            dag_proposers,
            dag_proposer_config,
            [0; 20],
            0,
        )
    }

    /// Constructs the production runtime with exact FinalChain execution policy.
    pub fn new_with_proposers_and_execution(
        signing_identities: Vec<SigningIdentity>,
        polling_interval_ms: u64,
        dag_proposers: Vec<DagProposerSessionBeginInput>,
        dag_proposer_config: DagProposerConfig,
        bridge_contract_address: [u8; 20],
        max_levels_per_period: u64,
    ) -> Result<Self> {
        ensure!(
            polling_interval_ms > 0,
            "CONSENSUS_RUNTIME_ZERO_POLLING_INTERVAL"
        );
        for (position, identity) in signing_identities.iter().enumerate() {
            ensure!(
                identity.wallet_index == position as u64,
                "CONSENSUS_RUNTIME_SIGNING_INDEX_NOT_DENSE"
            );
        }
        ensure!(
            dag_proposers.len() <= signing_identities.len(),
            "CONSENSUS_RUNTIME_DAG_PROPOSER_IDENTITY_MISSING"
        );
        for (position, proposer) in dag_proposers.iter().enumerate() {
            let identity = &signing_identities[position];
            ensure!(
                proposer.proposer_address == identity.address
                    && proposer.wallet_vrf_public_key == identity.vrf_public_key,
                "CONSENSUS_RUNTIME_DAG_PROPOSER_IDENTITY_MISMATCH"
            );
        }
        Ok(Self {
            generation: AtomicU64::new(0),
            sequence: AtomicU64::new(0),
            operation_sequence: AtomicU64::new(0),
            running: AtomicBool::new(false),
            signing_identities,
            polling_interval_ms,
            dag_proposers,
            dag_proposer_config,
            bridge_contract_address,
            max_levels_per_period,
            vdf_executor: NativeVdfExecutor::new(),
        })
    }

    /// Issues an identity for an operation-shaped leaf executed outside the
    /// blocking daemon loop.
    ///
    /// Generation zero is reserved for these calls and is never issued by a
    /// daemon run. The independent monotonic sequence prevents a concurrent RPC
    /// or network admission report from being mistaken for another operation.
    pub(crate) fn next_operation_effect(&self) -> Result<ConsensusEffectId> {
        let sequence = self
            .operation_sequence
            .fetch_add(1, Ordering::AcqRel)
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("CONSENSUS_OPERATION_EFFECT_SEQUENCE_EXHAUSTED"))?;
        Ok(ConsensusEffectId {
            generation: 0,
            sequence,
        })
    }

    /// Rejects a stale or reordered operation-shaped host report.
    pub(crate) fn validate_operation_report(
        &self,
        expected: ConsensusEffectId,
        actual: ConsensusEffectId,
    ) -> Result<()> {
        ensure!(
            expected == actual && actual.generation == 0,
            "CONSENSUS_OPERATION_STALE_EFFECT_REPORT"
        );
        Ok(())
    }

    pub fn signing_identities(&self) -> &[SigningIdentity] {
        &self.signing_identities
    }

    pub(crate) fn dag_gas_limit(&self, proposal_period: u64) -> u64 {
        self.dag_proposer_config.gas_limits(proposal_period).0
    }

    pub(crate) fn pbft_gas_limit(&self, proposal_period: u64) -> u64 {
        self.dag_proposer_config.gas_limits(proposal_period).1
    }

    fn begin_run(&self) -> Result<u64> {
        ensure!(
            !self.running.swap(true, Ordering::AcqRel),
            "CONSENSUS_RUNTIME_ALREADY_RUNNING"
        );
        let generation = self
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("CONSENSUS_RUNTIME_GENERATION_EXHAUSTED"))?;
        self.sequence.store(0, Ordering::Release);
        Ok(generation)
    }

    fn next_effect(&self, generation: u64) -> Result<ConsensusEffectId> {
        ensure!(
            self.running.load(Ordering::Acquire),
            "CONSENSUS_RUNTIME_NOT_RUNNING"
        );
        ensure!(
            self.generation.load(Ordering::Acquire) == generation,
            "CONSENSUS_RUNTIME_STALE_GENERATION"
        );
        let sequence = self
            .sequence
            .fetch_add(1, Ordering::AcqRel)
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("CONSENSUS_RUNTIME_EFFECT_SEQUENCE_EXHAUSTED"))?;
        Ok(ConsensusEffectId {
            generation,
            sequence,
        })
    }

    fn validate_report(
        &self,
        expected: ConsensusEffectId,
        actual: ConsensusEffectId,
    ) -> Result<()> {
        ensure!(expected == actual, "CONSENSUS_RUNTIME_STALE_EFFECT_REPORT");
        ensure!(
            actual.generation == self.generation.load(Ordering::Acquire),
            "CONSENSUS_RUNTIME_STALE_EFFECT_GENERATION"
        );
        Ok(())
    }

    fn wait_for<P: ConsensusProcessPort>(
        &self,
        generation: u64,
        delay_ms: u64,
        process: &P,
    ) -> Result<ConsensusWaitOutcome> {
        let id = self.next_effect(generation)?;
        let report = process.wait(&ConsensusWaitRequest {
            effect_id: id,
            delay_ms,
        })?;
        self.validate_report(id, report.effect_id)?;
        Ok(report.outcome)
    }

    #[allow(
        dead_code,
        reason = "used by the pending native vote-action composition"
    )]
    fn sign_digest<S: ConsensusSigningPort>(
        &self,
        generation: u64,
        wallet_index: u64,
        digest: [u8; 32],
        signer: &S,
    ) -> Result<Vec<u8>> {
        ensure!(
            self.signing_identities
                .get(wallet_index as usize)
                .is_some_and(|identity| identity.wallet_index == wallet_index),
            "CONSENSUS_RUNTIME_SIGNER_UNKNOWN"
        );
        let id = self.next_effect(generation)?;
        let report = signer.sign_digest(&ConsensusSignRequest {
            effect_id: id,
            wallet_index,
            digest,
        })?;
        self.validate_report(id, report.effect_id)?;
        ensure!(
            report.succeeded,
            "CONSENSUS_RUNTIME_SIGN_FAILED: {}",
            report.error_code
        );
        ensure!(
            !report.signature.is_empty(),
            "CONSENSUS_RUNTIME_EMPTY_SIGNATURE"
        );
        Ok(report.signature)
    }

    fn prove_vrf<S: ConsensusSigningPort>(
        &self,
        generation: u64,
        wallet_index: u64,
        message: Vec<u8>,
        signer: &S,
    ) -> Result<Vec<u8>> {
        let id = self.next_effect(generation)?;
        let report = signer.prove_vrf(&ConsensusVrfRequest {
            effect_id: id,
            wallet_index,
            message,
        })?;
        self.validate_report(id, report.effect_id)?;
        ensure!(
            report.succeeded,
            "CONSENSUS_RUNTIME_VRF_FAILED: {}",
            report.error_code
        );
        ensure!(
            !report.proof.is_empty(),
            "CONSENSUS_RUNTIME_EMPTY_VRF_PROOF"
        );
        Ok(report.proof)
    }

    #[allow(
        dead_code,
        reason = "used by the pending native vote-action composition"
    )]
    fn gossip_vote<T: ConsensusTransportPort>(
        &self,
        generation: u64,
        vote_rlp: Vec<u8>,
        proposed_block_rlp: Vec<u8>,
        rebroadcast: bool,
        transport: &T,
    ) -> Result<ConsensusTransportReport> {
        let id = self.next_effect(generation)?;
        let report = transport.gossip_vote(&GossipVoteRequest {
            effect_id: id,
            vote_rlp,
            proposed_block_rlp,
            rebroadcast,
        })?;
        self.validate_report(id, report.effect_id)?;
        Ok(report)
    }

    pub(crate) fn execute_final_chain_task<E: ConsensusExecutionPort>(
        &self,
        pbft: &PbftService,
        final_chain: &FinalChain,
        request: EvmFinalizationRequest,
        evm: &E,
    ) -> Result<crate::FinalChainApplicationExecutionReport> {
        let execution_request =
            crate::pbft_application_finalization::final_chain_execution_request_from_period_data(
                &request.period_data_rlp,
                &request.previous_cert_vote_rlps,
                &request.previous_cert_vote_weights,
                request.blocks_per_year,
                final_chain.block_gas_limit(),
            )?;
        let period = rustaxa_types::PbftBlockMetadata::try_from(
            rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp::new(
                &execution_request.pbft_block_rlp,
            ),
        )?
        .period;
        let period: rustaxa_types::FinalChainBlockNumber = period.into();
        let expected_next = final_chain
            .committed_state_descriptor()?
            .period
            .checked_next()
            .context("FINAL_CHAIN_RUNTIME_PRIOR_PERIOD_OVERFLOW")?;
        ensure!(
            period == expected_next,
            "FINAL_CHAIN_RUNTIME_NON_CONSECUTIVE_PERIOD: expected {}, requested {}",
            expected_next.as_u64(),
            period.as_u64()
        );
        final_chain.ensure_period_data(period, &request.period_data_rlp)?;
        let proposal_period_update = if request.anchor_block_rlp.is_empty() {
            crate::FinalChainProposalPeriodDagLevelUpdate::default()
        } else {
            let anchor = rustaxa_types::DagBlock::try_from(
                rustaxa_types::codec::rlp::dag::DagBlockRlp::new(&request.anchor_block_rlp),
            )?;
            crate::FinalChainProposalPeriodDagLevelUpdate {
                has_update: true,
                level: anchor
                    .level
                    .checked_add(self.max_levels_per_period)
                    .context("FINAL_CHAIN_PROPOSAL_PERIOD_LEVEL_OVERFLOW")?,
            }
        };
        let (ficus_activation, pillar_interval) = pbft.pillar_schedule();
        let system_period = period
            .as_u64()
            .checked_add(final_chain.dpos_delegation_delay())
            .context("FINAL_CHAIN_SYSTEM_PERIOD_OVERFLOW")?;
        let first_pillar_period = if ficus_activation == 0 {
            pillar_interval
        } else {
            ficus_activation
        };
        let is_pillar_block_period = ficus_activation != u64::MAX
            && pillar_interval > 0
            && system_period >= first_pillar_period
            && system_period % pillar_interval == 0;
        crate::execute_final_chain_application_task(
            final_chain,
            execution_request,
            proposal_period_update,
            is_pillar_block_period,
            self.bridge_contract_address,
            evm,
        )
    }

    fn load_account_facts(
        &self,
        addresses: Vec<[u8; 20]>,
        final_chain: &FinalChain,
    ) -> Result<NativeFinalChainAccountFacts> {
        let observed_block = final_chain.last_block_number_typed()?;
        let accounts = addresses
            .into_iter()
            .map(|address| {
                let account = final_chain.account_at_block(observed_block, address)?;
                Ok(match account {
                    Some(account) => NativeFinalChainAccountFact {
                        address,
                        found: true,
                        nonce: account.nonce.to_bytes(),
                        balance: account.balance.to_snapshot_bytes(),
                    },
                    None => NativeFinalChainAccountFact {
                        address,
                        found: false,
                        nonce: Vec::new(),
                        balance: Vec::new(),
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(NativeFinalChainAccountFacts {
            observed_block: observed_block.as_u64(),
            accounts,
        })
    }

    /// Composes one pillar anchor from native FinalChain state and the exact
    /// bridge-contract facts supplied by the concrete EVM leaf.
    ///
    /// Header/state-root and DPoS snapshots never cross CXX. The host observes
    /// only `period` and returns bridge root/epoch; missing native rows, corrupt
    /// RLP, or report identity mismatches fail before protocol state advances.
    fn load_pillar_anchor_state<E: ConsensusExecutionPort>(
        &self,
        effect_id: ConsensusEffectId,
        period: u64,
        pillar_block_period: u64,
        signer_addresses: Vec<[u8; 20]>,
        final_chain: &FinalChain,
        evm: &E,
    ) -> Result<PillarAnchorStateReport> {
        let mut report = evm.load_pillar_anchor_state(&PillarAnchorStateRequest {
            effect_id,
            period,
            pillar_block_period,
            signer_addresses: signer_addresses.clone(),
        })?;
        self.validate_report(effect_id, report.effect_id)?;
        if !report.succeeded {
            return Ok(report);
        }

        let block_header_rlp = final_chain
            .block_header(period.into())?
            .context("PILLAR_ANCHOR_HEADER_MISSING")?;
        let state_root = Rlp::new(&block_header_rlp)
            .val_at::<ethereum_types::H256>(3)
            .context("PILLAR_ANCHOR_STATE_ROOT_DECODE")?;
        report.block_header_rlp = block_header_rlp;
        report.state_root = state_root.into();
        report.validator_vote_counts = final_chain
            .dpos_validators_eligible_vote_counts(pillar_block_period.into())?
            .into_iter()
            .map(|count| PillarAnchorValidatorVoteCount {
                address: count.address,
                vote_count: count.vote_count,
            })
            .collect();
        report.signer_vote_counts = signer_addresses
            .into_iter()
            .map(|address| {
                final_chain.dpos_eligible_vote_count(pillar_block_period.into(), address)
            })
            .collect::<Result<Vec<_>>>()?;
        report.total_eligible_vote_count =
            final_chain.dpos_eligible_total_vote_count(pillar_block_period.into())?;
        Ok(report)
    }

    /// Publishes one already-committed pillar block to the public observer.
    ///
    /// Persistence and native finalized state have already been acknowledged
    /// before this method receives the event. Observer unavailability,
    /// rejection, or a malformed acknowledgement therefore cannot roll back or
    /// fail consensus; the event leaf is deliberately best effort.
    fn observe_finalized_pillar<O: ConsensusObserverPort>(
        &self,
        generation: u64,
        observation: Option<&PbftApplicationPillarObservation>,
        observer: &O,
    ) {
        let Some(observation) = observation else {
            return;
        };
        let Ok(effect_id) = self.next_effect(generation) else {
            return;
        };
        if let Ok(report) = observer.observe(&ConsensusObservationRequest {
            effect_id,
            kind: CONSENSUS_OBSERVATION_KIND_PILLAR_BLOCK,
            period: 0,
            hash: observation.block_hash,
            canonical_rlp: observation.block_data_rlp.clone(),
        }) {
            let _ = self.validate_report(effect_id, report.effect_id);
        }
    }

    /// Publishes an already-durable FinalChain block identity. Consumers load
    /// public block/transaction/receipt data through `ConsensusQueryApi`.
    fn observe_finalized_block<O: ConsensusObserverPort>(
        &self,
        generation: u64,
        report: &crate::FinalChainApplicationExecutionReport,
        observer: &O,
    ) {
        let Ok(effect_id) = self.next_effect(generation) else {
            return;
        };
        if let Ok(ack) = observer.observe(&ConsensusObservationRequest {
            effect_id,
            kind: CONSENSUS_OBSERVATION_KIND_FINALIZED_BLOCK,
            period: report.period.as_u64(),
            hash: report.block_hash,
            canonical_rlp: Vec::new(),
        }) {
            let _ = self.validate_report(effect_id, ack.effect_id);
        }
    }

    fn cancel_active_dag_vdf(&self, job_id: u64) -> Result<()> {
        self.vdf_executor.cancel(job_id)
    }

    /// Drives one complete native DAG proposal attempt for one configured wallet.
    #[allow(clippy::too_many_arguments)]
    fn drive_dag_proposer<P, S, T, E, O>(
        &self,
        generation: u64,
        wallet_index: u64,
        input: DagProposerSessionBeginInput,
        dag: &DagTransactionService,
        pbft: &PbftService,
        final_chain: &FinalChain,
        process: &P,
        signer: &S,
        transport: &T,
        execution: &E,
        observer: &O,
    ) -> Result<bool>
    where
        P: ConsensusProcessPort,
        S: ConsensusSigningPort,
        T: ConsensusTransportPort,
        E: ConsensusExecutionPort,
        O: ConsensusObserverPort,
    {
        let session_id = dag.begin_proposer_session(input)?;
        let mut active_vdf_job = None;
        let result = (|| {
            let mut step =
                dag.report_proposer_final_chain_facts_with_final_chain(session_id, final_chain)?;
            if matches!(step.action, DagProposerSessionAction::Complete) {
                return Ok(step.return_value);
            }
            ensure!(
                matches!(step.action, DagProposerSessionAction::ProveVrf),
                "CONSENSUS_DAG_PROPOSER_EXPECTED_VRF"
            );
            let vrf_id = self.next_effect(generation)?;
            let vrf = signer.prove_vrf(&ConsensusVrfRequest {
                effect_id: vrf_id,
                wallet_index,
                message: step.vrf_input.clone(),
            })?;
            self.validate_report(vrf_id, vrf.effect_id)?;
            ensure!(
                vrf.succeeded,
                "CONSENSUS_RUNTIME_VRF_FAILED: {}",
                vrf.error_code
            );
            step = dag.report_proposer_vrf(
                session_id,
                DagProposerVrfReport {
                    proof: vrf.proof,
                    output: vrf.output,
                },
            )?;
            if matches!(step.action, DagProposerSessionAction::Complete) {
                return Ok(step.return_value);
            }
            ensure!(
                matches!(step.action, DagProposerSessionAction::PackTransactions),
                "CONSENSUS_DAG_PROPOSER_EXPECTED_PACK"
            );
            let (dag_gas_limit, pbft_gas_limit) =
                self.dag_proposer_config.gas_limits(step.proposal_period);
            dag.configure_proposer_gas_policy(
                session_id,
                self.dag_proposer_config
                    .proposal_weight_limit(step.proposal_period),
                pbft_gas_limit,
                dag_gas_limit,
            )?;
            let pack = dag.prepare_proposer_pack(DagProposerPackPrepareRequest {
                session_id,
                network_throttled: {
                    let status = transport.transport_status();
                    pbft.network_service()
                        .pbft_sync_status(process.now_millis())?
                        .active
                        || status.packet_queue_over_limit
                },
                min_transaction_gas: 21_000,
                estimate_gas_limit: 200_000,
                last_block_number: final_chain.last_block_number()?,
            })?;
            step = pack.session;
            if !pack.estimate_requests.is_empty() {
                let effect_id = self.next_effect(generation)?;
                let report = execution.estimate_dag_transaction_gas(&DagGasEstimateRequest {
                    effect_id,
                    proposal_period: step.proposal_period,
                    transactions: pack
                        .estimate_requests
                        .iter()
                        .map(|value| DagGasEstimateInput {
                            hash: value.hash.0,
                            transaction_rlp: value.transaction_rlp.clone(),
                        })
                        .collect(),
                })?;
                self.validate_report(effect_id, report.effect_id)?;
                ensure!(
                    report.succeeded,
                    "CONSENSUS_DAG_GAS_FAILED: {}",
                    report.error_code
                );
                ensure!(
                    report.estimates.len() == pack.estimate_requests.len(),
                    "CONSENSUS_DAG_GAS_COUNT_MISMATCH"
                );
                step = dag
                    .finalize_proposer_pack(
                        session_id,
                        report
                            .estimates
                            .into_iter()
                            .map(|value| TransactionPackingEstimate {
                                hash: value.hash.into(),
                                gas_used: value.gas_used,
                                last_block_number: report.observed_block,
                                result_rlp: value.result_rlp,
                            })
                            .collect(),
                    )?
                    .session;
            }
            if matches!(step.action, DagProposerSessionAction::Complete) {
                return Ok(step.return_value);
            }
            ensure!(
                matches!(step.action, DagProposerSessionAction::StartVdf),
                "CONSENSUS_DAG_PROPOSER_EXPECTED_VDF"
            );
            let job_id = self.vdf_executor.start(NativeVdfRequest {
                vrf_proof: step.vrf_proof.clone(),
                vdf_message: step.vdf_message.clone(),
                lambda_bound: step.sortition_params.vdf.lambda_bound,
                difficulty: step.vdf_difficulty,
            })?;
            active_vdf_job = Some(job_id);
            let vdf_rlp = loop {
                let native = dag.poll_proposer_vdf(session_id)?;
                if matches!(native.action, DagProposerSessionAction::CancelVdf)
                    || process.stop_requested(generation)
                {
                    self.cancel_active_dag_vdf(job_id)?;
                    active_vdf_job = None;
                    let _ = dag.abort_proposer_session(session_id);
                    return Ok(native.return_value);
                }
                match self.vdf_executor.poll(job_id)? {
                    NativeVdfPollResult::Completed(vdf_rlp) => {
                        active_vdf_job = None;
                        break vdf_rlp;
                    }
                    NativeVdfPollResult::Cancelled => {
                        active_vdf_job = None;
                        bail!("CONSENSUS_DAG_VDF_UNEXPECTED_CANCELLATION");
                    }
                    NativeVdfPollResult::Pending => {}
                }
                if self.wait_for(
                    generation,
                    crate::dag::DAG_PROPOSER_VDF_POLL_INTERVAL_MS,
                    process,
                )? == ConsensusWaitOutcome::Stopped
                {
                    continue;
                }
            };
            step = dag.report_proposer_vdf_proof(
                session_id,
                DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp,
                },
            )?;
            if matches!(step.action, DagProposerSessionAction::StaleProofSleep) {
                let _ = self.wait_for(
                    generation,
                    crate::dag::DAG_PROPOSER_STALE_PROOF_SLEEP_MS,
                    process,
                )?;
                step = dag.resume_proposer_stale_proof(session_id)?;
            }
            if matches!(step.action, DagProposerSessionAction::Complete) {
                return Ok(step.return_value);
            }
            ensure!(
                matches!(step.action, DagProposerSessionAction::SignBlock),
                "CONSENSUS_DAG_PROPOSER_EXPECTED_SIGN"
            );
            let signature =
                self.sign_digest(generation, wallet_index, step.signing_hash.0, signer)?;
            step =
                dag.report_proposer_signing(session_id, DagProposerSigningReport { signature })?;
            ensure!(
                matches!(step.action, DagProposerSessionAction::AddBlock),
                "CONSENSUS_DAG_PROPOSER_EXPECTED_ADD"
            );
            let signed = step
                .signed_intent
                .clone()
                .context("CONSENSUS_DAG_SIGNED_INTENT_MISSING")?;
            let transactions = step
                .selected_transactions
                .iter()
                .map(|value| DagAddBlockTransactionPayload {
                    hash: value.hash,
                    transaction_rlp: value.transaction_rlp.clone(),
                })
                .collect();
            let prepared = dag.prepare_add_block(DagAddBlockPrepareRequest {
                expected_hash: signed.block_hash,
                block_rlp: signed.block_rlp.clone(),
                validate_hash: true,
                save: true,
                proposed: true,
                transactions,
            })?;
            let add = if prepared.cursor_id == 0 {
                DagProposerAddBlockReport {
                    accepted: prepared.accepted,
                    duplicate: prepared.duplicate,
                    expired: prepared.expired,
                    missing_references: prepared.missing_references,
                }
            } else {
                let accounts = self.load_account_facts(
                    prepared
                        .account_requests
                        .iter()
                        .map(|value| value.sender.0)
                        .collect(),
                    final_chain,
                )?;
                let commit = dag.complete_add_block(DagAddBlockCompletion {
                    cursor_id: prepared.cursor_id,
                    account_nonce_facts: accounts
                        .accounts
                        .into_iter()
                        .enumerate()
                        .map(|(idx, value)| DagAddBlockAccountNonceFact {
                            input_index: idx as u64,
                            account_nonce: ethereum_types::U256::from_big_endian(&value.nonce),
                        })
                        .collect(),
                })?;
                if commit.emit_verified {
                    let id = self.next_effect(generation)?;
                    if let Ok(report) = observer.observe(&ConsensusObservationRequest {
                        effect_id: id,
                        kind: 2,
                        period: 0,
                        hash: signed.block_hash.0,
                        canonical_rlp: signed.block_rlp.clone(),
                    }) {
                        self.validate_report(id, report.effect_id)?;
                    }
                }
                if commit.gossip {
                    let id = self.next_effect(generation)?;
                    if let Ok(report) = transport.gossip_dag_block(&GossipDagBlockRequest {
                        effect_id: id,
                        block_hash: signed.block_hash.0,
                        block_rlp: signed.block_rlp.clone(),
                    }) {
                        self.validate_report(id, report.effect_id)?;
                    }
                }
                DagProposerAddBlockReport {
                    accepted: commit.accepted,
                    duplicate: false,
                    expired: false,
                    missing_references: Vec::new(),
                }
            };
            step = dag.report_proposer_add_block(session_id, add)?;
            ensure!(
                matches!(step.action, DagProposerSessionAction::Complete),
                "CONSENSUS_DAG_PROPOSER_NOT_COMPLETE"
            );
            Ok(step.return_value)
        })();
        if result.is_err() {
            if let Some(job_id) = active_vdf_job.take() {
                let _ = self.cancel_active_dag_vdf(job_id);
            }
            let _ = dag.abort_proposer_session(session_id);
        }
        result
    }

    fn drive_application_finalization<E, S, T, O>(
        &self,
        generation: u64,
        request: &PbftApplicationFinalizationRequest,
        mut step: PbftApplicationFinalizationStep,
        pbft: &PbftService,
        dag: &DagTransactionService,
        final_chain: &FinalChain,
        signer: &S,
        transport: &T,
        evm: &E,
        observer: &O,
    ) -> Result<bool>
    where
        E: ConsensusExecutionPort,
        S: ConsensusSigningPort,
        T: ConsensusTransportPort,
        O: ConsensusObserverPort,
    {
        loop {
            step = match step {
                PbftApplicationFinalizationStep::Complete(_) => return Ok(true),
                PbftApplicationFinalizationStep::Rejected { .. } => return Ok(false),
                PbftApplicationFinalizationStep::AccountFacts(effect) => {
                    let report = self.load_account_facts(effect.addresses.clone(), final_chain)?;
                    report_pbft_application_finalization_account_facts(
                        pbft,
                        dag,
                        final_chain,
                        request,
                        &effect,
                        PbftApplicationAccountFactsReport {
                            cursor: effect.cursor,
                            succeeded: true,
                            observed_block: report.observed_block,
                            accounts: report
                                .accounts
                                .into_iter()
                                .map(|account| PbftApplicationAccountFact {
                                    address: account.address,
                                    found: account.found,
                                    nonce: ethereum_types::U256::from_big_endian(&account.nonce)
                                        .to_big_endian(),
                                })
                                .collect(),
                            error_code: String::new(),
                        },
                    )?
                }
                PbftApplicationFinalizationStep::PillarAnchor(effect) => {
                    let id = self.next_effect(generation)?;
                    let signer_addresses = self
                        .signing_identities
                        .iter()
                        .map(|identity| identity.address)
                        .collect();
                    let report = self.load_pillar_anchor_state(
                        id,
                        effect.period,
                        effect.pillar_block_period,
                        signer_addresses,
                        final_chain,
                        evm,
                    )?;
                    let succeeded = report.succeeded;
                    let failure = report.error_code.clone();
                    let identities = self
                        .signing_identities
                        .iter()
                        .map(|identity| (identity.wallet_index, identity.address))
                        .collect::<Vec<_>>();
                    let next = report_pbft_application_finalization_pillar_anchor(
                        pbft,
                        dag,
                        final_chain,
                        request,
                        &identities,
                        PbftApplicationPillarAnchorReport {
                            cursor: effect.cursor,
                            succeeded: report.succeeded,
                            block_header_rlp: report.block_header_rlp,
                            state_root: report.state_root,
                            bridge_root: report.bridge_root,
                            bridge_epoch: report.bridge_epoch,
                            validator_vote_counts: report
                                .validator_vote_counts
                                .into_iter()
                                .map(|count| crate::pillar_chain::PillarValidatorVoteCount {
                                    address: count.address.into(),
                                    vote_count: count.vote_count,
                                })
                                .collect(),
                            signer_vote_counts: report.signer_vote_counts,
                            total_eligible_vote_count: report.total_eligible_vote_count,
                            error_code: report.error_code,
                        },
                    )?;
                    ensure!(
                        succeeded,
                        "CONSENSUS_RUNTIME_PILLAR_ANCHOR_FAILED: {failure}"
                    );
                    next
                }
                PbftApplicationFinalizationStep::PillarSign(signing) => {
                    let signature = self.sign_digest(
                        generation,
                        signing.draft.wallet_index,
                        signing.draft.digest.0,
                        signer,
                    )?;
                    report_pbft_application_finalization_pillar_signature(
                        pbft,
                        dag,
                        final_chain,
                        request,
                        signing,
                        signature,
                    )?
                }
                PbftApplicationFinalizationStep::PillarGossip(gossip) => {
                    let id = self.next_effect(generation)?;
                    let report = transport.gossip_pillar_vote(&GossipPillarVoteRequest {
                        effect_id: id,
                        pillar_vote_rlp: gossip.pillar_vote_rlp.clone(),
                        rebroadcast: false,
                    })?;
                    self.validate_report(id, report.effect_id)?;
                    let succeeded = report.succeeded;
                    let failure = report.error_code.clone();
                    let next = report_pbft_application_finalization_pillar_gossip(
                        pbft,
                        dag,
                        final_chain,
                        request,
                        gossip,
                        report.succeeded,
                        u8::from(!report.succeeded),
                        report.error_code,
                    )?;
                    ensure!(
                        succeeded,
                        "CONSENSUS_RUNTIME_PILLAR_GOSSIP_FAILED: {failure}"
                    );
                    next
                }
                PbftApplicationFinalizationStep::Evm(effect) => {
                    let report = self.execute_final_chain_task(
                        pbft,
                        final_chain,
                        EvmFinalizationRequest {
                            effect_id: ConsensusEffectId::default(),
                            period_data_rlp: effect.period_data_rlp,
                            previous_cert_vote_rlps: effect.previous_cert_vote_rlps,
                            previous_cert_vote_weights: Vec::new(),
                            finalized_dag_hashes: effect.finalized_dag_hashes,
                            blocks_per_year: effect.blocks_per_year,
                            synchronous: effect.synchronous,
                            anchor_block_rlp: effect.anchor_block_rlp,
                        },
                        evm,
                    )?;
                    let succeeded = report.error_code.is_empty();
                    let failure = report.error_code.clone();
                    if succeeded {
                        self.observe_finalized_block(generation, &report, observer);
                    }
                    let next = report_pbft_application_finalization_evm(
                        pbft,
                        dag,
                        final_chain,
                        request,
                        PbftApplicationEvmReport {
                            cursor: effect.cursor,
                            succeeded,
                            status: report.status,
                            last_block_number: report.period.as_u64(),
                            error_code: report.error_code,
                        },
                    )?;
                    ensure!(
                        succeeded,
                        "CONSENSUS_RUNTIME_FINALIZATION_FAILED: {failure}"
                    );
                    next
                }
            };
        }
    }

    fn slashing_submitters(
        &self,
        final_chain: &FinalChain,
    ) -> Result<Vec<crate::SlashingSubmitterIdentity>> {
        let addresses = self
            .signing_identities
            .iter()
            .map(|identity| identity.address)
            .collect::<Vec<_>>();
        let report = self.load_account_facts(addresses, final_chain)?;
        Ok(self
            .signing_identities
            .iter()
            .zip(report.accounts)
            .map(|(identity, account)| crate::SlashingSubmitterIdentity {
                wallet_index: identity.wallet_index as usize,
                address: identity.address,
                nonce: if account.found {
                    ethereum_types::U256::from_big_endian(&account.nonce)
                } else {
                    ethereum_types::U256::zero()
                },
                balance: if account.found {
                    ethereum_types::U256::from_big_endian(&account.balance)
                } else {
                    ethereum_types::U256::zero()
                },
            })
            .collect())
    }

    fn execute_slashing_effect<P, S, T>(
        &self,
        generation: u64,
        effect: &crate::SlashingTransactionEffect,
        pbft: &PbftService,
        dag: &DagTransactionService,
        final_chain: &FinalChain,
        process: &P,
        signer: &S,
        transport: &T,
        submitters: &[crate::SlashingSubmitterIdentity],
    ) -> Result<()>
    where
        P: ConsensusProcessPort,
        S: ConsensusSigningPort,
        T: ConsensusTransportPort,
    {
        let leaves = RuntimeSyncedLeaves {
            runtime: self,
            generation,
            pbft,
            final_chain,
            process,
            signer,
            transport,
        };
        let inserted =
            pbft.submit_synced_slashing_transaction(effect, dag, final_chain, submitters, &leaves)?;
        let report =
            pbft.report_verified_vote_slashing_transaction_submission(effect.proof_hash, inserted)?;
        ensure!(
            report.submitted == inserted,
            "CONSENSUS_RUNTIME_SLASHING_REPORT_MISMATCH"
        );
        Ok(())
    }

    /// Dispatches one scheduled gossip batch and validates every host report.
    ///
    /// Exact report identities and batch invariants are terminal. A typed host
    /// rejection returns `None`, leaving cadence counters unchanged so the next
    /// daemon tick retries the same eligible payload family.
    fn dispatch_scheduled_gossip<T: ConsensusTransportPort>(
        &self,
        generation: u64,
        batch: &MaybeBroadcastVotesBatch,
        transport: &T,
    ) -> Result<Option<MaybeBroadcastVotesCommit>> {
        let mut acknowledgements = Vec::with_capacity(batch.requests.len());
        for request in &batch.requests {
            let host_id = self.next_effect(generation)?;
            let report = match request {
                ConsensusVoteTransportRequest::Vote {
                    canonical_vote_rlp,
                    proposed_block_rlp,
                    rebroadcast,
                    ..
                } => transport.gossip_vote(&GossipVoteRequest {
                    effect_id: host_id,
                    vote_rlp: canonical_vote_rlp.clone(),
                    proposed_block_rlp: proposed_block_rlp.clone().unwrap_or_default(),
                    rebroadcast: *rebroadcast,
                })?,
                ConsensusVoteTransportRequest::VoteBundle {
                    canonical_votes_bundle_rlp,
                    rebroadcast,
                    ..
                } => transport.gossip_vote_bundle(&GossipVoteBundleRequest {
                    effect_id: host_id,
                    votes_bundle_rlp: canonical_votes_bundle_rlp.clone(),
                    rebroadcast: *rebroadcast,
                })?,
                ConsensusVoteTransportRequest::PillarVote {
                    canonical_pillar_vote_rlp,
                    rebroadcast,
                    ..
                } => transport.gossip_pillar_vote(&GossipPillarVoteRequest {
                    effect_id: host_id,
                    pillar_vote_rlp: canonical_pillar_vote_rlp.clone(),
                    rebroadcast: *rebroadcast,
                })?,
            };
            self.validate_report(host_id, report.effect_id)?;
            acknowledgements.push(VoteBroadcastAcknowledgement {
                request_id: request.request_id(),
                family: request.family(),
                succeeded: report.succeeded,
                error_code: report.error_code,
            });
        }
        validate_maybe_broadcast_votes_acknowledgements(batch, &acknowledgements)
    }

    /// Runs the native daemon on the calling thread until stop or failure.
    ///
    /// The loop consumes the existing native manager tick session. Timing,
    /// round advancement, lifecycle transitions, and vote-broadcast selection,
    /// acknowledgement, and counter publication are native. Remaining state
    /// action and finalization variants are listed explicitly and fail closed
    /// until their composed service operations are available.
    pub fn run<P, S, T, E, O>(
        &self,
        pbft: &PbftService,
        dag_transaction: &DagTransactionService,
        final_chain: &FinalChain,
        process: &P,
        signer: &S,
        transport: &T,
        evm: &E,
        observer: &O,
    ) -> Result<ConsensusRunExit>
    where
        P: ConsensusProcessPort,
        S: ConsensusSigningPort,
        T: ConsensusTransportPort,
        E: ConsensusExecutionPort,
        O: ConsensusObserverPort,
    {
        let generation = self.begin_run()?;
        struct RunningGuard<'a>(&'a AtomicBool);
        impl Drop for RunningGuard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _guard = RunningGuard(&self.running);
        if !pbft.is_ready() {
            let signing_addresses: Vec<_> = self
                .signing_identities
                .iter()
                .map(|identity| identity.address)
                .collect();
            let startup =
                prepare_consensus_startup(pbft, dag_transaction, final_chain, &signing_addresses)?;
            for request in startup.finalizations.iter().cloned() {
                let report = self.execute_final_chain_task(pbft, final_chain, request, evm)?;
                ensure!(
                    report.error_code.is_empty(),
                    "CONSENSUS_RUNTIME_STARTUP_FINALIZATION_FAILED: {}",
                    report.error_code
                );
                self.observe_finalized_block(generation, &report, observer);
            }
            ensure!(
                pbft.finalization_ready(final_chain)?,
                "CONSENSUS_STARTUP_FINALIZATION_NOT_READY"
            );
            hydrate_recently_finalized_transactions(&startup, dag_transaction)?;
            if let Some(persisted) = &startup.persisted_pillar_vote {
                loop {
                    if process.stop_requested(generation) {
                        return Ok(ConsensusRunExit {
                            generation,
                            reason: ConsensusRunReason::Stopped,
                        });
                    }
                    let effect_id = self.next_effect(generation)?;
                    let report = self.load_pillar_anchor_state(
                        effect_id,
                        persisted.dpos_period,
                        persisted.dpos_period,
                        vec![persisted.voter],
                        final_chain,
                        evm,
                    );
                    let facts = startup_persisted_pillar_vote_report(report)?;
                    match facts {
                        StartupPersistedPillarVoteFacts::Ready {
                            validator_vote_count,
                            total_eligible_vote_count,
                        } => {
                            apply_startup_persisted_pillar_vote(
                                persisted,
                                validator_vote_count,
                                total_eligible_vote_count,
                                pbft,
                            )?;
                            break;
                        }
                        StartupPersistedPillarVoteFacts::Retry => {
                            if self.wait_for(generation, self.polling_interval_ms, process)?
                                == ConsensusWaitOutcome::Stopped
                            {
                                return Ok(ConsensusRunExit {
                                    generation,
                                    reason: ConsensusRunReason::Stopped,
                                });
                            }
                        }
                    }
                }
            }
            if let Some(period) = startup.pillar_anchor_state_period {
                let effect_id = self.next_effect(generation)?;
                let signer_addresses = self
                    .signing_identities
                    .iter()
                    .map(|identity| identity.address)
                    .collect();
                let report = self.load_pillar_anchor_state(
                    effect_id,
                    period,
                    startup.current_period,
                    signer_addresses,
                    final_chain,
                    evm,
                )?;
                ensure!(
                    report.succeeded,
                    "CONSENSUS_RUNTIME_PILLAR_ANCHOR_STATE_FAILED: {}",
                    report.error_code
                );
                let identities: Vec<_> = self
                    .signing_identities
                    .iter()
                    .map(|identity| (identity.wallet_index, identity.address))
                    .collect();
                let drafts =
                    apply_startup_pillar_anchor_state(&startup, &report, pbft, &identities)?;
                for draft in drafts {
                    let signature = self.sign_digest(
                        generation,
                        draft.wallet_index,
                        draft.digest.into(),
                        signer,
                    )?;
                    let vote_rlp = apply_startup_pillar_vote(&draft, &signature, pbft)?;
                    let effect_id = self.next_effect(generation)?;
                    let report = transport.gossip_pillar_vote(&GossipPillarVoteRequest {
                        effect_id,
                        pillar_vote_rlp: vote_rlp,
                        rebroadcast: false,
                    })?;
                    self.validate_report(effect_id, report.effect_id)?;
                    ensure!(
                        report.succeeded,
                        "CONSENSUS_RUNTIME_STARTUP_PILLAR_GOSSIP_FAILED: {}",
                        report.error_code
                    );
                }
            }
            complete_consensus_startup(pbft)?;
        }
        let mut tick_id = 0u64;
        let mut timing = ConsensusTimingOrigins::new(process.now_millis());
        loop {
            if process.stop_requested(generation) {
                pbft.abort_runtime_session();
                return Ok(ConsensusRunExit {
                    generation,
                    reason: ConsensusRunReason::Stopped,
                });
            }
            for (wallet_index, proposer) in self.dag_proposers.iter().cloned().enumerate() {
                self.drive_dag_proposer(
                    generation,
                    wallet_index as u64,
                    proposer,
                    dag_transaction,
                    pbft,
                    final_chain,
                    process,
                    signer,
                    transport,
                    evm,
                    observer,
                )?;
            }
            tick_id = tick_id
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("CONSENSUS_RUNTIME_TICK_EXHAUSTED"))?;
            let snapshot = pbft.manager_snapshot();
            timing.observe(snapshot.period, snapshot.round, process.now_millis());
            let transport_status = transport.transport_status();
            let network_sync_status = pbft
                .network_service()
                .pbft_sync_status(process.now_millis())?;
            let addresses: Vec<_> = self
                .signing_identities
                .iter()
                .map(|id| id.address)
                .collect();
            let has_eligible_wallet = final_chain
                .pbft_dpos_eligible_wallet_vote_counts(snapshot.period, &addresses)?
                .is_some_and(|votes| votes.into_iter().any(|vote| vote.vote_count > 0));
            pbft.begin_runtime_session(PbftManagerRuntimeTickFact {
                tick_id,
                state: snapshot.state,
                period: snapshot.period,
                round: snapshot.round,
                step: snapshot.step,
                network_available: transport_status.available,
                network_pbft_syncing: network_sync_status.active,
                has_eligible_wallet,
                polling_interval_ms: self.polling_interval_ms,
            });
            loop {
                let step = pbft
                    .runtime_session_next()
                    .ok_or_else(|| anyhow::anyhow!("CONSENSUS_RUNTIME_SESSION_MISSING"))?;
                if step.complete || step.status == PbftManagerRuntimeStatus::Complete {
                    break;
                }
                ensure!(
                    step.status == PbftManagerRuntimeStatus::Active,
                    "CONSENSUS_RUNTIME_SESSION_REJECTED: {}",
                    step.error_code
                );
                let action = step
                    .action
                    .ok_or_else(|| anyhow::anyhow!("CONSENSUS_RUNTIME_ACTION_MISSING"))?;
                let mut has_new_round = false;
                let mut new_round = 0;
                let mut go_finish_state = false;
                let mut loop_back_finish_state = false;
                let result = match action {
                    PbftManagerRuntimeAction::SleepIneligiblePollingInterval => {
                        if self.wait_for(generation, step.sleep_ms, process)?
                            == ConsensusWaitOutcome::Stopped
                        {
                            pbft.abort_runtime_session();
                            return Ok(ConsensusRunExit {
                                generation,
                                reason: ConsensusRunReason::Stopped,
                            });
                        }
                        PbftManagerRuntimeActionResultCode::SleepApplied
                    }
                    PbftManagerRuntimeAction::SleepUntilNextStep => {
                        let elapsed_ms = timing.round_elapsed_ms(process.now_millis());
                        let elapsed_ms = i64::try_from(elapsed_ms).map_err(|_| {
                            anyhow::anyhow!("CONSENSUS_RUNTIME_ROUND_ELAPSED_OVERFLOW")
                        })?;
                        let plan = pbft.plan_runtime_sleep_until_next_step(elapsed_ms);
                        ensure!(
                            plan.accepted,
                            "CONSENSUS_RUNTIME_SLEEP_PLAN_REJECTED: {}",
                            plan.error_code
                        );
                        if plan.should_sleep
                            && self.wait_for(generation, plan.sleep_ms, process)?
                                == ConsensusWaitOutcome::Stopped
                        {
                            pbft.abort_runtime_session();
                            return Ok(ConsensusRunExit {
                                generation,
                                reason: ConsensusRunReason::Stopped,
                            });
                        }
                        PbftManagerRuntimeActionResultCode::SleepApplied
                    }
                    PbftManagerRuntimeAction::DelayCertifyPoll
                    | PbftManagerRuntimeAction::DelayFinishPoll => {
                        let outcome = if action == PbftManagerRuntimeAction::DelayCertifyPoll {
                            pbft.delay_certify_poll()?
                        } else {
                            pbft.delay_finish_poll()?
                        };
                        ensure!(
                            outcome.status == PbftLifecycleActionStatus::Applied,
                            "CONSENSUS_RUNTIME_DELAY_REJECTED: {}",
                            outcome.error_code
                        );
                        if self.wait_for(generation, self.polling_interval_ms, process)?
                            == ConsensusWaitOutcome::Stopped
                        {
                            pbft.abort_runtime_session();
                            return Ok(ConsensusRunExit {
                                generation,
                                reason: ConsensusRunReason::Stopped,
                            });
                        }
                        PbftManagerRuntimeActionResultCode::SleepApplied
                    }
                    PbftManagerRuntimeAction::TryAdvanceRound => {
                        if let PbftRoundAdvanceActionOutcome::AdvanceTo(outcome) =
                            pbft.try_advance_round()?
                        {
                            has_new_round = true;
                            new_round = outcome.new_round;
                        }
                        PbftManagerRuntimeActionResultCode::NoProgressContinue
                    }
                    PbftManagerRuntimeAction::MaybeBroadcastVotes => {
                        let live = pbft.manager_snapshot();
                        let now = process.now_millis();
                        if let Some(batch) = select_maybe_broadcast_votes(
                            pbft,
                            MaybeBroadcastVotesInput {
                                action_id: MaybeBroadcastVotesActionId(tick_id),
                                period: live.period,
                                round: live.round,
                                round_elapsed_ms: timing.round_elapsed_ms(now),
                                period_elapsed_ms: timing.period_elapsed_ms(now),
                                current_round_lambda_ms: live.current_round_lambda_ms,
                                broadcast_lambda_threshold: 20,
                                rebroadcast_lambda_threshold: 60,
                                counters: VoteBroadcastCounters {
                                    broadcast_votes: live.broadcast_votes_counter,
                                    rebroadcast_votes: live.rebroadcast_votes_counter,
                                    broadcast_reward_votes: live.broadcast_reward_votes_counter,
                                    rebroadcast_reward_votes: live.rebroadcast_reward_votes_counter,
                                },
                            },
                        )? {
                            let commit =
                                self.dispatch_scheduled_gossip(generation, &batch, transport)?;
                            if let Some(commit) = commit {
                                pbft.apply_broadcast_counters_if_epoch(
                                    live.period,
                                    live.round,
                                    commit.counters.broadcast_votes,
                                    commit.counters.rebroadcast_votes,
                                    commit.counters.broadcast_reward_votes,
                                    commit.counters.rebroadcast_reward_votes,
                                );
                            }
                        }
                        PbftManagerRuntimeActionResultCode::StateActionDone
                    }
                    PbftManagerRuntimeAction::TryPushCertVotesBlock => {
                        let live = pbft.manager_snapshot();
                        match pbft.verified_votes_get_two_t_plus_one_voted_block_payloads(
                            live.period,
                            live.round,
                            TwoTPlusOneVotedBlockType::CertVotedBlock,
                        )? {
                            None => PbftManagerRuntimeActionResultCode::NoProgressContinue,
                            Some(certified) => {
                                match pbft.proposed_block(live.period, certified.block_hash) {
                                    None => PbftManagerRuntimeActionResultCode::NoProgressContinue,
                                    Some(proposed) if !proposed.is_valid => {
                                        PbftManagerRuntimeActionResultCode::NoProgressContinue
                                    }
                                    Some(proposed) => {
                                        let (request, prepared) =
                                            prepare_certified_pbft_application_finalization(
                                                pbft,
                                                dag_transaction,
                                                final_chain,
                                                proposed.block_rlp,
                                                certified
                                                    .votes
                                                    .into_iter()
                                                    .map(|vote| vote.vote_rlp)
                                                    .collect(),
                                                // The application root advances the PBFT period as
                                                // soon as this exact leaf reports success. Waiting
                                                // here preserves the legacy finalization barrier
                                                // without reintroducing a manager-shaped wait port.
                                                true,
                                            )?;
                                        self.observe_finalized_pillar(
                                            generation,
                                            prepared.pillar_observation.as_ref(),
                                            observer,
                                        );
                                        if self.drive_application_finalization(
                                            generation,
                                            &request,
                                            prepared.step,
                                            pbft,
                                            dag_transaction,
                                            final_chain,
                                            signer,
                                            transport,
                                            evm,
                                            observer,
                                        )? {
                                            PbftManagerRuntimeActionResultCode::ProgressRestartLoop
                                        } else {
                                            PbftManagerRuntimeActionResultCode::NoProgressContinue
                                        }
                                    }
                                }
                            }
                        }
                    }
                    PbftManagerRuntimeAction::ResetConsensus
                    | PbftManagerRuntimeAction::TransitionToFilter
                    | PbftManagerRuntimeAction::TransitionToCertify
                    | PbftManagerRuntimeAction::TransitionToFinish
                    | PbftManagerRuntimeAction::TransitionToFinishPolling
                    | PbftManagerRuntimeAction::LoopBackFinish => {
                        let outcome = match action {
                            PbftManagerRuntimeAction::ResetConsensus => {
                                ensure!(
                                    step.has_target_round && step.target_round > 0,
                                    "CONSENSUS_RUNTIME_RESET_TARGET_MISSING"
                                );
                                pbft.reset_consensus(step.target_round)?
                            }
                            PbftManagerRuntimeAction::TransitionToFilter => {
                                pbft.transition_to_filter()?
                            }
                            PbftManagerRuntimeAction::TransitionToCertify => {
                                pbft.transition_to_certify()?
                            }
                            PbftManagerRuntimeAction::TransitionToFinish => {
                                pbft.transition_to_finish()?
                            }
                            PbftManagerRuntimeAction::TransitionToFinishPolling => {
                                pbft.transition_to_finish_polling()?
                            }
                            PbftManagerRuntimeAction::LoopBackFinish => pbft.loop_back_finish()?,
                            _ => unreachable!(),
                        };
                        ensure!(
                            outcome.status == PbftLifecycleActionStatus::Applied,
                            "CONSENSUS_RUNTIME_LIFECYCLE_REJECTED: {}",
                            outcome.error_code
                        );
                        PbftManagerRuntimeActionResultCode::TransitionApplied
                    }
                    PbftManagerRuntimeAction::RunValueProposal => {
                        let live = pbft.manager_snapshot();
                        let mut eligible = Vec::new();
                        for identity in &self.signing_identities {
                            let request =
                                prepare_public_proposer_vrf(PbftPublicProposerSortitionInput {
                                    wallet_index: identity.wallet_index,
                                    pbft_period: live.period,
                                    pbft_round: live.round,
                                    vrf_public_key: identity.vrf_public_key,
                                    voter_public_key: identity.node_public_key,
                                    voter: ethereum_types::H160(identity.address),
                                })?;
                            let proof = self.prove_vrf(
                                generation,
                                identity.wallet_index,
                                request.message.clone(),
                                signer,
                            )?;
                            let proposer = pbft.complete_public_proposer_sortition(
                                final_chain,
                                request,
                                proof,
                            )?;
                            if proposer.accepted {
                                eligible.push(identity.wallet_index);
                            }
                        }
                        let prepare_proposal_vote = |wallet_index: u64,
                                                     block_rlp: Vec<u8>|
                         -> Result<
                            Option<(u64, Vec<u8>, PbftGeneratedVote, ConsensusStateVoteTask)>,
                        > {
                            let identity = self
                                .signing_identities
                                .get(wallet_index as usize)
                                .filter(|identity| identity.wallet_index == wallet_index)
                                .ok_or_else(|| {
                                    anyhow::anyhow!("CONSENSUS_PROPOSAL_SIGNER_MISSING")
                                })?;
                            let link = rustaxa_types::pbft::PbftBlockLink::try_from(
                                rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp::new(
                                    &block_rlp,
                                ),
                            )?;
                            let vrf = prepare_pbft_vote_vrf(PbftVoteGenerationPublicInput {
                                wallet_index,
                                block_hash: link.block_hash,
                                vote_type: PbftVoteType::Propose,
                                period: live.period,
                                round: live.round,
                                step: 1,
                                voter: ethereum_types::H160(identity.address),
                                voter_public_key: identity.node_public_key,
                                vrf_public_key: identity.vrf_public_key,
                            })?;
                            let proof = self.prove_vrf(
                                generation,
                                wallet_index,
                                vrf.message.clone(),
                                signer,
                            )?;
                            let Some(signing) =
                                pbft.prepare_state_vote_signing(final_chain, vrf, proof)?
                            else {
                                return Ok(None);
                            };
                            let signature = self.sign_digest(
                                generation,
                                wallet_index,
                                signing.signing_hash.0,
                                signer,
                            )?;
                            let generated = complete_pbft_vote_signing(signing, signature)?;
                            let task = ConsensusStateVoteTask {
                                period: live.period,
                                round: live.round,
                                step: 1,
                                vote_type: PbftVoteType::Propose,
                                block_hash: link.block_hash,
                                proposed_block_rlp: block_rlp.clone(),
                                commit: ConsensusStateVoteCommit::None,
                            };
                            Ok(Some((wallet_index, block_rlp, generated, task)))
                        };
                        let action = compose_value_proposal(
                            pbft,
                            final_chain,
                            dag_transaction,
                            process.unix_time_seconds(),
                            eligible,
                        )?;
                        let gossip_rewards = |bundle: Vec<u8>| -> Result<()> {
                            if bundle.is_empty() {
                                return Ok(());
                            }
                            let effect_id = self.next_effect(generation)?;
                            let report =
                                transport.gossip_vote_bundle(&GossipVoteBundleRequest {
                                    effect_id,
                                    votes_bundle_rlp: bundle,
                                    rebroadcast: false,
                                })?;
                            self.validate_report(effect_id, report.effect_id)?;
                            ensure!(
                                report.succeeded,
                                "CONSENSUS_PROPOSAL_REWARD_GOSSIP_FAILED: {}",
                                report.error_code
                            );
                            Ok(())
                        };
                        let mut candidates = Vec::new();
                        let mut reward_votes_bundle_rlp = Vec::new();
                        let mut publish_selected_block = false;
                        match action {
                            ConsensusValueProposalAction::NoWork => {}
                            ConsensusValueProposalAction::Repropose {
                                eligible_wallet_indices,
                                block_rlp,
                                reward_votes_bundle_rlp: rewards,
                            } => {
                                reward_votes_bundle_rlp = rewards;
                                for wallet_index in eligible_wallet_indices {
                                    if let Some(candidate) =
                                        prepare_proposal_vote(wallet_index, block_rlp.clone())?
                                    {
                                        candidates.push(candidate);
                                    }
                                }
                            }
                            ConsensusValueProposalAction::Build {
                                eligible_wallet_indices,
                                unsigned,
                                reward_votes_bundle_rlp: rewards,
                            } => {
                                reward_votes_bundle_rlp = rewards;
                                publish_selected_block = true;
                                for wallet_index in eligible_wallet_indices {
                                    let signature = self.sign_digest(
                                        generation,
                                        wallet_index,
                                        unsigned.signing_hash,
                                        signer,
                                    )?;
                                    let block_rlp = complete_value_proposal_signing(
                                        unsigned.clone(),
                                        signature,
                                    )?;
                                    let metadata = rustaxa_types::PbftBlockMetadata::try_from(
                                        rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp::new(
                                            &block_rlp,
                                        ),
                                    )?;
                                    ensure!(
                                        metadata.author.0
                                            == self.signing_identities[wallet_index as usize]
                                                .address,
                                        "CONSENSUS_VALUE_PROPOSAL_SIGNER_MISMATCH"
                                    );
                                    if let Some(candidate) =
                                        prepare_proposal_vote(wallet_index, block_rlp)?
                                    {
                                        candidates.push(candidate);
                                    }
                                }
                            }
                        }
                        if !candidates.is_empty() {
                            let selected_candidates = if publish_selected_block {
                                let policy = pbft.value_proposal_admission_request(
                                    live.period,
                                    ethereum_types::H256::zero(),
                                );
                                let selection = pbft.select_local_proposal_candidate(
                                    final_chain,
                                    dag_transaction,
                                    PbftLocalProposalSelectionRequest {
                                        candidates: candidates
                                            .iter()
                                            .map(|(_, block_rlp, generated, _)| {
                                                PbftLocalProposalCandidate {
                                                    block_rlp: block_rlp.clone(),
                                                    vote_rlp: generated.vote_rlp.clone(),
                                                }
                                            })
                                            .collect(),
                                        period: live.period,
                                        round: live.round,
                                        pbft_gas_limit: policy.pbft_gas_limit,
                                        extra_data_required: policy.extra_data_required,
                                        pillar_block_required: policy.pillar_block_required,
                                    },
                                )?;
                                if !selection.selected {
                                    Vec::new()
                                } else {
                                    vec![
                                        candidates
                                            .into_iter()
                                            .nth(selection.selected_index as usize)
                                            .ok_or_else(|| {
                                                anyhow::anyhow!(
                                                    "CONSENSUS_PROPOSAL_SELECTED_INDEX_INVALID"
                                                )
                                            })?,
                                    ]
                                }
                            } else {
                                // A later-round re-proposal keeps the certified block fixed and
                                // contributes every eligible local wallet's fresh proposal vote.
                                candidates
                            };
                            if !selected_candidates.is_empty() {
                                gossip_rewards(reward_votes_bundle_rlp)?;
                                for (_, block_rlp, generated, task) in selected_candidates {
                                    if publish_selected_block {
                                        pbft.publish_proposed_block_effect(block_rlp.clone())?;
                                    }
                                    let resolved_submitters = RefCell::new(None);
                                    let admission = pbft.admit_state_vote_with_slashing_resolver(
                                        final_chain,
                                        &task,
                                        &generated,
                                        || {
                                            let submitters =
                                                self.slashing_submitters(final_chain)?;
                                            resolved_submitters.replace(Some(submitters.clone()));
                                            Ok(submitters)
                                        },
                                    )?;
                                    ensure!(
                                        admission.transaction.transition_published,
                                        "CONSENSUS_PROPOSAL_VOTE_ADMISSION_NOT_PUBLISHED: {}",
                                        admission.transaction.persistence_error_code
                                    );
                                    if let Some(effect) =
                                        admission.slashing_transaction_effect.as_ref()
                                    {
                                        let submitters = resolved_submitters.borrow();
                                        let submitters =
                                            submitters.as_deref().ok_or_else(|| {
                                                anyhow::anyhow!(
                                                    "CONSENSUS_RUNTIME_SLASHING_SUBMITTERS_MISSING"
                                                )
                                            })?;
                                        self.execute_slashing_effect(
                                            generation,
                                            effect,
                                            pbft,
                                            dag_transaction,
                                            final_chain,
                                            process,
                                            signer,
                                            transport,
                                            submitters,
                                        )?;
                                    }
                                    if admission.validation.accepted {
                                        // Admission is durable before transport. A failed gossip must
                                        // not retry with a new timestamp and create a conflicting vote;
                                        // periodic own-vote broadcast owns eventual retransmission.
                                        let _report = self.gossip_vote(
                                            generation,
                                            generated.vote_rlp,
                                            block_rlp,
                                            false,
                                            transport,
                                        )?;
                                    }
                                }
                            }
                        }
                        PbftManagerRuntimeActionResultCode::StateActionDone
                    }
                    PbftManagerRuntimeAction::RunFilter
                    | PbftManagerRuntimeAction::RunCertify
                    | PbftManagerRuntimeAction::RunFirstFinish
                    | PbftManagerRuntimeAction::RunSecondFinish => {
                        let batch = compose_consensus_state_action(
                            pbft,
                            final_chain,
                            dag_transaction,
                            ConsensusStateActionRequest {
                                round_elapsed_ms: timing.round_elapsed_ms(process.now_millis()),
                            },
                        )?;
                        go_finish_state = batch.go_finish_state;
                        loop_back_finish_state = batch.loop_back_finish_state;
                        for task in batch.votes {
                            for identity in &self.signing_identities {
                                let vrf = prepare_pbft_vote_vrf(PbftVoteGenerationPublicInput {
                                    wallet_index: identity.wallet_index,
                                    block_hash: task.block_hash,
                                    vote_type: task.vote_type,
                                    period: task.period,
                                    round: task.round,
                                    step: task.step,
                                    voter: ethereum_types::H160(identity.address),
                                    voter_public_key: identity.node_public_key,
                                    vrf_public_key: identity.vrf_public_key,
                                })?;
                                let proof = self.prove_vrf(
                                    generation,
                                    identity.wallet_index,
                                    vrf.message.clone(),
                                    signer,
                                )?;
                                let Some(signing) =
                                    pbft.prepare_state_vote_signing(final_chain, vrf, proof)?
                                else {
                                    continue;
                                };
                                let signature = self.sign_digest(
                                    generation,
                                    identity.wallet_index,
                                    signing.signing_hash.0,
                                    signer,
                                )?;
                                let generated = complete_pbft_vote_signing(signing, signature)?;
                                let resolved_submitters = RefCell::new(None);
                                let admission = pbft.admit_state_vote_with_slashing_resolver(
                                    final_chain,
                                    &task,
                                    &generated,
                                    || {
                                        let submitters = self.slashing_submitters(final_chain)?;
                                        resolved_submitters.replace(Some(submitters.clone()));
                                        Ok(submitters)
                                    },
                                )?;
                                ensure!(
                                    admission.transaction.transition_published,
                                    "CONSENSUS_STATE_VOTE_ADMISSION_NOT_PUBLISHED: {}",
                                    admission.transaction.persistence_error_code
                                );
                                if let Some(effect) = admission.slashing_transaction_effect.as_ref()
                                {
                                    let submitters = resolved_submitters.borrow();
                                    let submitters = submitters.as_deref().ok_or_else(|| {
                                        anyhow::anyhow!(
                                            "CONSENSUS_RUNTIME_SLASHING_SUBMITTERS_MISSING"
                                        )
                                    })?;
                                    self.execute_slashing_effect(
                                        generation,
                                        effect,
                                        pbft,
                                        dag_transaction,
                                        final_chain,
                                        process,
                                        signer,
                                        transport,
                                        submitters,
                                    )?;
                                }
                                if admission.validation.accepted {
                                    // The admitted vote is durable. Transport rejection is retained
                                    // as a typed report and periodic own-vote broadcast retries it.
                                    let _report = self.gossip_vote(
                                        generation,
                                        generated.vote_rlp,
                                        task.proposed_block_rlp.clone(),
                                        false,
                                        transport,
                                    )?;
                                }
                            }
                            pbft.commit_state_vote_task(&task)?;
                        }
                        PbftManagerRuntimeActionResultCode::StateActionDone
                    }
                    PbftManagerRuntimeAction::ProcessSyncedPbftBlocks => {
                        let leaves = RuntimeSyncedLeaves {
                            runtime: self,
                            generation,
                            pbft,
                            final_chain,
                            process,
                            signer,
                            transport,
                        };
                        let outcome = match pbft.process_synced_pbft_blocks(
                            dag_transaction,
                            final_chain,
                            || self.slashing_submitters(final_chain),
                            &leaves,
                            |accepted| {
                                let request = PbftApplicationFinalizationRequest {
                                    period_data_rlp: accepted.period_data_rlp,
                                    current_cert_vote_rlps: accepted.current_cert_vote_rlps,
                                    synchronous: true,
                                };
                                let prepared = prepare_pbft_application_finalization(
                                    pbft,
                                    dag_transaction,
                                    final_chain,
                                    request.clone(),
                                )?;
                                self.observe_finalized_pillar(
                                    generation,
                                    prepared.pillar_observation.as_ref(),
                                    observer,
                                );
                                self.drive_application_finalization(
                                    generation,
                                    &request,
                                    prepared.step,
                                    pbft,
                                    dag_transaction,
                                    final_chain,
                                    signer,
                                    transport,
                                    evm,
                                    observer,
                                )
                            },
                        ) {
                            Ok(outcome) => outcome,
                            Err(_) if process.stop_requested(generation) => {
                                pbft.abort_runtime_session();
                                return Ok(ConsensusRunExit {
                                    generation,
                                    reason: ConsensusRunReason::Stopped,
                                });
                            }
                            Err(error) => return Err(error),
                        };
                        if outcome.finalized_entries > 0 {
                            PbftManagerRuntimeActionResultCode::ProgressRestartLoop
                        } else {
                            PbftManagerRuntimeActionResultCode::StateActionDone
                        }
                    }
                    PbftManagerRuntimeAction::Unknown => {
                        bail!("CONSENSUS_RUNTIME_UNKNOWN_ACTION: {action:?}")
                    }
                };
                let reported = pbft
                    .report_runtime_session(PbftManagerRuntimeActionReport {
                        cursor: step.cursor,
                        action,
                        success: true,
                        result,
                        go_finish_state,
                        loop_back_finish_state,
                        has_eligible_wallet,
                        has_new_round,
                        new_round,
                        error_code: String::new(),
                    })
                    .ok_or_else(|| anyhow::anyhow!("CONSENSUS_RUNTIME_REPORT_SESSION_MISSING"))?;
                ensure!(
                    reported.status == PbftManagerRuntimeStatus::Active
                        || reported.status == PbftManagerRuntimeStatus::Complete,
                    "CONSENSUS_RUNTIME_REPORT_REJECTED: {}",
                    reported.error_code
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbft_manager::{PbftManagerSleepFact, plan_pbft_manager_sleep_until_next_step};
    use std::sync::Mutex;

    struct FakeProcess {
        outcome: ConsensusWaitOutcome,
        stale: bool,
    }

    struct RecordingObserver {
        requests: Mutex<Vec<ConsensusObservationRequest>>,
        fail: bool,
        stale: bool,
    }

    impl ConsensusObserverPort for RecordingObserver {
        fn observe(
            &self,
            request: &ConsensusObservationRequest,
        ) -> Result<ConsensusObservationReport> {
            self.requests.lock().unwrap().push(request.clone());
            if self.fail {
                bail!("INJECTED_OBSERVER_FAILURE");
            }
            Ok(ConsensusObservationReport {
                effect_id: ConsensusEffectId {
                    generation: request.effect_id.generation,
                    sequence: request.effect_id.sequence + u64::from(self.stale),
                },
                succeeded: !self.stale,
                error_code: self
                    .stale
                    .then_some("INJECTED_STALE_OBSERVER_REPORT".to_owned())
                    .unwrap_or_default(),
            })
        }
    }

    #[test]
    fn period_advance_resets_round_epoch_for_lambda_sleep_plan() {
        let mut timing = ConsensusTimingOrigins::new(100);
        timing.observe(1, 1, 100);
        assert_eq!(timing.round_elapsed_ms(400), 300);

        timing.observe(2, 1, 450);
        assert_eq!(timing.period_elapsed_ms(450), 0);
        assert_eq!(timing.round_elapsed_ms(450), 0);

        let plan = plan_pbft_manager_sleep_until_next_step(PbftManagerSleepFact {
            next_step_time_ms: 500,
            round_elapsed_ms: i64::try_from(timing.round_elapsed_ms(450)).unwrap(),
            step: 2,
        });
        assert!(plan.accepted);
        assert!(plan.should_sleep);
        assert_eq!(plan.sleep_ms, 500);
    }

    #[test]
    fn unavailable_persisted_pillar_vote_facts_require_retry() {
        let unavailable = PillarAnchorStateReport {
            effect_id: ConsensusEffectId {
                generation: 1,
                sequence: 2,
            },
            succeeded: false,
            block_header_rlp: Vec::new(),
            state_root: [0; 32],
            bridge_root: [0; 32],
            bridge_epoch: [0; 32],
            validator_vote_counts: Vec::new(),
            signer_vote_counts: Vec::new(),
            total_eligible_vote_count: 0,
            error_code: "DPOS_SNAPSHOT_UNAVAILABLE".into(),
        };
        assert_eq!(
            startup_persisted_pillar_vote_facts(&unavailable).unwrap(),
            StartupPersistedPillarVoteFacts::Retry
        );

        let mut ready = unavailable;
        ready.succeeded = true;
        ready.signer_vote_counts = vec![7];
        ready.total_eligible_vote_count = 11;
        assert_eq!(
            startup_persisted_pillar_vote_facts(&ready).unwrap(),
            StartupPersistedPillarVoteFacts::Ready {
                validator_vote_count: 7,
                total_eligible_vote_count: 11,
            }
        );

        ready.signer_vote_counts.push(8);
        assert!(startup_persisted_pillar_vote_facts(&ready).is_err());
    }

    #[test]
    fn persisted_pillar_startup_propagates_stale_effect_reports() {
        for code in [
            "CONSENSUS_RUNTIME_STALE_EFFECT_REPORT",
            "CONSENSUS_RUNTIME_STALE_EFFECT_GENERATION",
        ] {
            let error = anyhow::anyhow!(code).context("PERSISTED_PILLAR_STARTUP");
            assert_eq!(
                startup_persisted_pillar_vote_report(Err(error))
                    .unwrap_err()
                    .root_cause()
                    .to_string(),
                code
            );
        }

        assert_eq!(
            startup_persisted_pillar_vote_report(Err(anyhow::anyhow!(
                "PILLAR_ANCHOR_STATE_READ_FAILED"
            )))
            .unwrap(),
            StartupPersistedPillarVoteFacts::Retry
        );
    }
    impl ConsensusProcessPort for FakeProcess {
        fn now_millis(&self) -> u64 {
            7
        }
        fn unix_time_seconds(&self) -> u64 {
            7
        }
        fn wait(&self, request: &ConsensusWaitRequest) -> Result<ConsensusWaitReport> {
            Ok(ConsensusWaitReport {
                effect_id: ConsensusEffectId {
                    generation: request.effect_id.generation,
                    sequence: request.effect_id.sequence + u64::from(self.stale),
                },
                outcome: self.outcome,
            })
        }
        fn stop_requested(&self, _generation: u64) -> bool {
            self.outcome == ConsensusWaitOutcome::Stopped
        }
    }

    struct FakeSigner {
        succeed: bool,
    }
    impl ConsensusSigningPort for FakeSigner {
        fn sign_digest(&self, request: &ConsensusSignRequest) -> Result<ConsensusSignReport> {
            Ok(ConsensusSignReport {
                effect_id: request.effect_id,
                succeeded: self.succeed,
                signature: self.succeed.then_some(vec![9; 65]).unwrap_or_default(),
                error_code: (!self.succeed)
                    .then_some("SIGN_REJECTED".into())
                    .unwrap_or_default(),
            })
        }
        fn prove_vrf(&self, request: &ConsensusVrfRequest) -> Result<ConsensusVrfReport> {
            Ok(ConsensusVrfReport {
                effect_id: request.effect_id,
                succeeded: false,
                proof: Vec::new(),
                output: Vec::new(),
                error_code: "UNUSED".into(),
            })
        }
    }

    struct FakeTransport {
        succeed: bool,
        stale: bool,
    }

    struct FailOnceTransport {
        calls: std::sync::atomic::AtomicUsize,
    }

    impl FakeTransport {
        fn report(&self, effect_id: ConsensusEffectId) -> ConsensusTransportReport {
            ConsensusTransportReport {
                effect_id: ConsensusEffectId {
                    generation: effect_id.generation,
                    sequence: effect_id.sequence + u64::from(self.stale),
                },
                succeeded: self.succeed,
                error_code: (!self.succeed)
                    .then_some("TRANSPORT_REJECTED".into())
                    .unwrap_or_default(),
            }
        }
    }
    impl ConsensusTransportPort for FakeTransport {
        fn gossip_vote(&self, request: &GossipVoteRequest) -> Result<ConsensusTransportReport> {
            Ok(self.report(request.effect_id))
        }
        fn gossip_vote_bundle(
            &self,
            request: &GossipVoteBundleRequest,
        ) -> Result<ConsensusTransportReport> {
            Ok(self.report(request.effect_id))
        }
        fn gossip_pillar_vote(
            &self,
            request: &GossipPillarVoteRequest,
        ) -> Result<ConsensusTransportReport> {
            Ok(self.report(request.effect_id))
        }
        fn transport_status(&self) -> ConsensusTransportStatus {
            ConsensusTransportStatus {
                available: true,
                packet_queue_over_limit: false,
            }
        }
        fn report_malicious_peer(
            &self,
            request: &ReportMaliciousPeerRequest,
        ) -> Result<ConsensusTransportReport> {
            Ok(self.report(request.effect_id))
        }
    }

    impl ConsensusTransportPort for FailOnceTransport {
        fn gossip_vote(&self, request: &GossipVoteRequest) -> Result<ConsensusTransportReport> {
            let call = self.calls.fetch_add(1, Ordering::AcqRel);
            Ok(ConsensusTransportReport {
                effect_id: request.effect_id,
                succeeded: call > 0,
                error_code: (call == 0)
                    .then_some("TRANSPORT_REJECTED".to_owned())
                    .unwrap_or_default(),
            })
        }

        fn gossip_vote_bundle(
            &self,
            _request: &GossipVoteBundleRequest,
        ) -> Result<ConsensusTransportReport> {
            unreachable!("test batch contains only one vote request")
        }

        fn gossip_pillar_vote(
            &self,
            _request: &GossipPillarVoteRequest,
        ) -> Result<ConsensusTransportReport> {
            unreachable!("test batch contains only one vote request")
        }

        fn transport_status(&self) -> ConsensusTransportStatus {
            ConsensusTransportStatus {
                available: true,
                packet_queue_over_limit: false,
            }
        }

        fn report_malicious_peer(
            &self,
            _request: &ReportMaliciousPeerRequest,
        ) -> Result<ConsensusTransportReport> {
            unreachable!("scheduled gossip never reports peers")
        }
    }

    fn identity() -> SigningIdentity {
        SigningIdentity {
            wallet_index: 0,
            address: [1; 20],
            node_public_key: [2; 64],
            vrf_public_key: [3; 32],
        }
    }

    #[test]
    fn generations_restart_and_effect_reports_are_exact() {
        let runtime = ConsensusApplicationRuntime::new(vec![identity()], 100).unwrap();
        let first = runtime.begin_run().unwrap();
        let effect = runtime.next_effect(first).unwrap();
        assert!(
            runtime
                .validate_report(
                    effect,
                    ConsensusEffectId {
                        generation: first,
                        sequence: effect.sequence + 1
                    }
                )
                .is_err()
        );
        runtime.running.store(false, Ordering::Release);
        let second = runtime.begin_run().unwrap();
        assert_eq!(second, first + 1);
        assert!(runtime.validate_report(effect, effect).is_err());
    }

    #[test]
    fn finalized_pillar_observer_uses_kind_three_and_is_best_effort() {
        let runtime = ConsensusApplicationRuntime::new(vec![identity()], 100).unwrap();
        let generation = runtime.begin_run().unwrap();
        let observation = PbftApplicationPillarObservation {
            block_hash: [7; 32],
            block_data_rlp: vec![0xc2, 0x80, 0x80],
        };
        for (fail, stale) in [(false, false), (true, false), (false, true)] {
            let observer = RecordingObserver {
                requests: Mutex::new(Vec::new()),
                fail,
                stale,
            };
            runtime.observe_finalized_pillar(generation, Some(&observation), &observer);
            let requests = observer.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].kind, CONSENSUS_OBSERVATION_KIND_PILLAR_BLOCK);
            assert_eq!(requests[0].hash, observation.block_hash);
            assert_eq!(requests[0].canonical_rlp, observation.block_data_rlp);
        }
    }

    #[test]
    fn finalized_block_observer_uses_kind_four_identity_and_is_best_effort() {
        let runtime = ConsensusApplicationRuntime::new(vec![identity()], 100).unwrap();
        let generation = runtime.begin_run().unwrap();
        let report = crate::FinalChainApplicationExecutionReport {
            period: 7u64.into(),
            block_hash: [8; 32],
            ..Default::default()
        };
        for (fail, stale) in [(false, false), (true, false), (false, true)] {
            let observer = RecordingObserver {
                requests: Mutex::new(Vec::new()),
                fail,
                stale,
            };
            runtime.observe_finalized_block(generation, &report, &observer);
            let requests = observer.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].kind, CONSENSUS_OBSERVATION_KIND_FINALIZED_BLOCK);
            assert_eq!(requests[0].period, 7);
            assert_eq!(requests[0].hash, report.block_hash);
            assert!(requests[0].canonical_rlp.is_empty());
        }
    }

    #[test]
    fn signing_inventory_is_public_dense_and_secret_free() {
        let runtime = ConsensusApplicationRuntime::new(vec![identity()], 1).unwrap();
        assert_eq!(runtime.signing_identities()[0].address, [1; 20]);
        let mut invalid = identity();
        invalid.wallet_index = 9;
        assert!(ConsensusApplicationRuntime::new(vec![invalid], 1).is_err());
    }

    #[test]
    fn wait_stop_and_stale_identity_are_validated() {
        let runtime = ConsensusApplicationRuntime::new(vec![identity()], 1).unwrap();
        let generation = runtime.begin_run().unwrap();
        assert_eq!(
            runtime
                .wait_for(
                    generation,
                    5,
                    &FakeProcess {
                        outcome: ConsensusWaitOutcome::Stopped,
                        stale: false
                    }
                )
                .unwrap(),
            ConsensusWaitOutcome::Stopped
        );
        assert!(
            runtime
                .wait_for(
                    generation,
                    5,
                    &FakeProcess {
                        outcome: ConsensusWaitOutcome::Elapsed,
                        stale: true
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn signing_and_post_admission_transport_semantics_are_exact() {
        let runtime = ConsensusApplicationRuntime::new(vec![identity()], 1).unwrap();
        let generation = runtime.begin_run().unwrap();
        assert_eq!(
            runtime
                .sign_digest(generation, 0, [4; 32], &FakeSigner { succeed: true })
                .unwrap()
                .len(),
            65
        );
        assert!(
            runtime
                .sign_digest(generation, 0, [4; 32], &FakeSigner { succeed: false })
                .is_err()
        );
        let rejected = runtime
            .gossip_vote(
                generation,
                vec![1],
                Vec::new(),
                false,
                &FakeTransport {
                    succeed: false,
                    stale: false,
                },
            )
            .unwrap();
        assert!(!rejected.succeeded);
        assert_eq!(rejected.error_code, "TRANSPORT_REJECTED");

        assert!(
            runtime
                .gossip_vote(
                    generation,
                    vec![1],
                    Vec::new(),
                    false,
                    &FakeTransport {
                        succeed: true,
                        stale: true,
                    },
                )
                .is_err()
        );
    }

    #[test]
    fn scheduled_gossip_rejection_retries_without_committing_counters() {
        use crate::maybe_broadcast_votes::{
            MaybeBroadcastVotesActionId, VoteBroadcastFamily, VoteBroadcastRequestId,
        };

        let runtime = ConsensusApplicationRuntime::new(vec![identity()], 1).unwrap();
        let generation = runtime.begin_run().unwrap();
        let expected = VoteBroadcastCounters {
            broadcast_votes: 1,
            rebroadcast_votes: 0,
            broadcast_reward_votes: 0,
            rebroadcast_reward_votes: 0,
        };
        let batch = MaybeBroadcastVotesBatch {
            requests: vec![ConsensusVoteTransportRequest::Vote {
                request_id: VoteBroadcastRequestId {
                    action_id: MaybeBroadcastVotesActionId(9),
                    ordinal: 1,
                },
                family: VoteBroadcastFamily::OwnVote,
                canonical_vote_rlp: vec![0xc0],
                proposed_block_rlp: None,
                rebroadcast: false,
            }],
            next_counters: expected,
        };
        let transport = FailOnceTransport {
            calls: std::sync::atomic::AtomicUsize::new(0),
        };

        assert_eq!(
            runtime
                .dispatch_scheduled_gossip(generation, &batch, &transport)
                .unwrap(),
            None
        );
        assert_eq!(
            runtime
                .dispatch_scheduled_gossip(generation, &batch, &transport)
                .unwrap()
                .unwrap()
                .counters,
            expected
        );
        assert_eq!(transport.calls.load(Ordering::Acquire), 2);
    }
}
