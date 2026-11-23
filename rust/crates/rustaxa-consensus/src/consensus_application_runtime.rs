//! Native blocking lifecycle and host-leaf boundary for PBFT consensus.
//!
//! This module owns restart generations, effect identities, daemon scheduling,
//! and validation of host reports. Host ports expose only OS process mechanics,
//! key custody, physical transport, and concrete external-EVM execution. They
//! never receive a manager action or mutable consensus object.

/// Borrowed service owners used during one consensus run.
/// This value only groups existing borrows; readiness, generation and effect
/// checks remain in the runtime methods, with no new defaults or authority.
pub struct ConsensusRuntimeServices<'a> {
    /// PBFT protocol and finalization service.
    pub pbft: &'a PbftService,
    /// DAG transaction service used by the same application.
    pub dag: &'a DagTransactionService,
    /// Existing FinalChain owner used by this run.
    pub final_chain: &'a FinalChain,
}

struct FinalizationPorts<'a, S, T, E, O> {
    signer: &'a S,
    transport: &'a T,
    evm: &'a E,
    observer: &'a O,
}

struct SlashingPorts<'a, P, S, T> {
    process: &'a P,
    signer: &'a S,
    transport: &'a T,
}

#[derive(Debug)]
struct DagProposerStale;

impl std::fmt::Display for DagProposerStale {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str("PRODUCTION_DAG_PROPOSER_STALE")
    }
}

impl std::error::Error for DagProposerStale {}

struct PreparedProposalVote {
    block_rlp: Vec<u8>,
    vote: PbftGeneratedVote,
    task: ConsensusStateVoteTask,
    execution: Option<CheckedProposalExecution>,
}

enum BoundStateVote {
    Signed(PbftGeneratedVote),
    Retained(Vec<u8>),
}

enum OwnedVoteTransport {
    Completed(u64),
    Stale,
    Incomplete,
}

use crate::FinalChain;
use crate::consensus_application::DagProposerConfig;
use crate::consensus_application::production_application::{
    ApplicationComposition, ApplicationOperationScope, ApplicationScopeSource,
    CheckedProposalExecution, ProposalEgressPacket, ProposalExecutionCheck,
};
use crate::consensus_application_startup::{
    apply_startup_persisted_pillar_vote, apply_startup_pillar_anchor_state,
    apply_startup_pillar_vote, complete_consensus_startup, hydrate_recently_finalized_transactions,
    prepare_consensus_startup,
};
use crate::consensus_state_actions::{
    ConsensusStateActionRequest, ConsensusStateVoteCommit, ConsensusStateVoteTask,
    compose_bound_state_vote, compose_consensus_state_action,
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
    PbftManagerRuntimeStateCode, PbftManagerRuntimeStatus, PbftManagerRuntimeTickFact,
};
use crate::pbft_service::{
    PbftLifecycleActionStatus, PbftLocalProposalCandidate, PbftLocalProposalSelectionRequest,
    PbftProcessSyncedLeaves, PbftRoundAdvanceActionOutcome, PbftService,
};
use crate::pbft_vote_generation::{
    PbftGeneratedVote, PbftVoteGenerationPublicInput, PbftVoteVrfRequest,
    complete_pbft_vote_signing, prepare_pbft_vote_vrf,
};
use crate::pbft_vote_validation::{
    PbftPublicProposerSortitionInput, inspect_canonical_pbft_vote, prepare_public_proposer_vrf,
};
use crate::transaction_packing_service::TransactionPackingEstimate;
use crate::vdf_executor::{NativeVdfExecutor, NativeVdfPollResult, NativeVdfRequest};
use crate::verified_votes::PbftVoteType;
use crate::verified_votes::TwoTPlusOneVotedBlockType;
use anyhow::{Context, Result, bail, ensure};
use rlp::Rlp;
use std::cell::{Cell, RefCell};
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeWaitOutcome {
    Elapsed(Option<u64>),
    Stopped,
    StaleWork,
}

impl From<ConsensusWaitOutcome> for RuntimeWaitOutcome {
    fn from(value: ConsensusWaitOutcome) -> Self {
        match value {
            ConsensusWaitOutcome::Elapsed => Self::Elapsed(None),
            ConsensusWaitOutcome::Stopped => Self::Stopped,
        }
    }
}

struct RuntimeSessionGuard<'a> {
    pbft: &'a PbftService,
    composition: &'a ApplicationComposition,
    identity: crate::pbft_service::OwnedRuntimeSession,
    discarded: Cell<bool>,
}

impl RuntimeSessionGuard<'_> {
    fn discard(&self) -> Result<()> {
        let _operation = self.composition.mutation()?;
        ensure!(
            self.pbft.discard_owned_runtime_session(&self.identity),
            "CONSENSUS_RUNTIME_SESSION_REPLACED"
        );
        self.discarded.set(true);
        Ok(())
    }
}

impl Drop for RuntimeSessionGuard<'_> {
    fn drop(&mut self) {
        if !self.discarded.get() {
            if let Ok(_operation) = self.composition.mutation() {
                self.pbft.discard_owned_runtime_session(&self.identity);
            }
        }
    }
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
    finish_poll_started_ms: u64,
}

impl ConsensusTimingOrigins {
    fn new(now_ms: u64) -> Self {
        Self {
            period: 0,
            round: 0,
            period_started_ms: now_ms,
            round_started_ms: now_ms,
            finish_poll_started_ms: now_ms,
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

    fn state_action_timing(self, now_ms: u64) -> ConsensusStateActionRequest {
        ConsensusStateActionRequest {
            round_elapsed_ms: self.round_elapsed_ms(now_ms),
            finish_poll_elapsed_ms: now_ms.saturating_sub(self.finish_poll_started_ms),
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

pub(crate) enum OwnedDagGasOrigin<'a> {
    Pack(u64),
    Verify {
        scope: &'a ApplicationOperationScope<'a>,
        cursor_id: u64,
        block_hash: ethereum_types::H256,
    },
}

/// Exact requests from a retained native packing or verification cursor.
pub struct OwnedDagGasRequest<'a> {
    pub(crate) source: &'a ApplicationScopeSource,
    pub(crate) generation: u64,
    pub(crate) committed_period: u64,
    pub(crate) origin: OwnedDagGasOrigin<'a>,
    pub(crate) request: &'a DagGasEstimateRequest,
}

/// Settled gas for each signed transaction in the production packing batch.
pub struct OwnedDagGasReport {
    pub effect_id: ConsensusEffectId,
    pub estimates: Vec<OwnedDagGasEstimate>,
}

pub struct OwnedDagGasEstimate {
    pub hash: [u8; 32],
    pub gas_used: u64,
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
pub struct ScheduledCertificate<'a> {
    pub period: u64,
    pub round: u64,
    pub block_rlp: &'a [u8],
    pub votes: &'a [crate::pbft_vote_payload::PbftVotePayloadRecord],
    pub owner_generation: u64,
}

/// A certified queue head admitted by the Rust runtime in the selected owner generation.
#[derive(Clone)]
pub struct ScheduledSyncedCertificate {
    pub(crate) queue: crate::period_data_queue::PeriodDataQueuePopPlan,
    pub(crate) round: u64,
    pub(crate) step: u64,
    pub(crate) votes: Vec<crate::pbft_vote_payload::PbftVotePayloadRecord>,
    pub(crate) owner_generation: u64,
}

/// A reached hard error in execution of the scheduled certified candidate.
#[derive(Debug)]
pub struct ScheduledCertificateRejected(pub String);

impl std::fmt::Display for ScheduledCertificateRejected {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(output, "rejected scheduled PBFT candidate: {}", self.0)
    }
}

impl std::error::Error for ScheduledCertificateRejected {}

/// Deterministic rejection of a queued candidate before publication starts.
#[derive(Debug)]
pub struct SyncedCertificateRejected(pub String);

impl std::fmt::Display for SyncedCertificateRejected {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(output, "rejected PBFT sync candidate: {}", self.0)
    }
}

impl std::error::Error for SyncedCertificateRejected {}

/// Concrete execution leaves. Production publication remains owner-bound.
pub trait ConsensusExecutionPort {
    /// Checks a signed proposal within the owner generation that produced it.
    fn check_proposal_transactions(
        &self,
        _block_rlp: &[u8],
        _owner_generation: u64,
    ) -> Result<Option<ProposalExecutionCheck>> {
        bail!("PRODUCTION_PROPOSAL_EXECUTION_UNAVAILABLE")
    }

    fn publish_scheduled_certificate(&self, _selection: ScheduledCertificate<'_>) -> Result<bool> {
        bail!("PRODUCTION_EXECUTION_PLAN_UNAVAILABLE: certified preparation before writes")
    }

    fn publish_synced_certificate(&self, _selection: &ScheduledSyncedCertificate) -> Result<bool> {
        bail!("PRODUCTION_EXECUTION_PLAN_UNAVAILABLE: synced certificate")
    }

    /// No transaction report can authorize certificate-dependent rewards or
    /// primary storage. A real full transition and owner reservation are needed.
    fn validate_certified_pbft_transition(
        &self,
        _admission: &crate::pbft_execution_guard::PbftExecutionAdmission,
        _canonical_period_data: &[u8],
    ) -> Result<crate::pbft_execution_guard::CertifiedPbftTransitionValidation> {
        Ok(
            crate::pbft_execution_guard::CertifiedPbftTransitionValidation::Unavailable(
                crate::pbft_execution_validation::PbftExecutionUnavailable::ExecutorUnavailable,
            ),
        )
    }

    /// Executes the exact candidate privately before non-null vote authority.
    /// An owner without the qualified Rust probe cannot approve a candidate.
    fn validate_pbft_execution(
        &self,
        request: &crate::pbft_execution_validation::PbftExecutionRequest,
    ) -> Result<crate::pbft_execution_validation::PbftExecutionValidation> {
        use crate::pbft_execution_validation::{PbftExecutionUnavailable, PbftExecutionValidation};
        Ok(PbftExecutionValidation::Unavailable {
            request_digest: request.digest(),
            reason: PbftExecutionUnavailable::ExecutorUnavailable,
        })
    }

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

    /// Applies the rewards boundary with an opaque application-owned native plan.
    /// Legacy implementations retain the existing request-only operation. A
    /// staged native adapter borrows the plan to finish its unpublished session;
    /// the application retains all runtime and storage publication authority.
    fn distribute_final_chain_rewards_with_native_plan(
        &self,
        request: &crate::FinalChainEvmRewardsRequest,
        _plan: &crate::final_chain_execution::FinalChainPreparedExternalEvmRewardsStatsPlan,
    ) -> Result<crate::FinalChainEvmRewardsReport> {
        self.distribute_final_chain_rewards(request)
    }

    /// Supplies the optional Rust invocation context after rewards preparation.
    /// The exact request/projection binding is validated before durable intent.
    /// Staged-native implementations must fail if context is unavailable; legacy
    /// leaves preserve their existing projection path by returning `None`.
    fn final_chain_native_projection_context(
        &self,
        _request_id: [u8; 32],
        _projection_hash: [u8; 32],
    ) -> Result<Option<crate::native_projection_context::FinalChainNativeProjectionContext>> {
        Ok(None)
    }

    /// Returns trusted lifecycle evidence independently from context claims.
    fn final_chain_native_lifecycle_authority(
        &self,
        _request_id: [u8; 32],
        _projection_hash: [u8; 32],
    ) -> Result<Option<crate::native_projection_context::FinalChainLifecycleAuthority>> {
        Ok(None)
    }

    fn final_chain_native_delayed_view(
        &self,
        _request_id: [u8; 32],
        _projection_hash: [u8; 32],
    ) -> Result<Option<std::rc::Rc<crate::native_session::frozen_eligibility::FrozenEligibility>>>
    {
        Ok(None)
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

    /// Estimates a batch issued by the production packing cursor. None means stale work.
    fn estimate_owned_dag_transaction_gas(
        &self,
        _request: OwnedDagGasRequest<'_>,
    ) -> Result<Option<OwnedDagGasReport>> {
        bail!("PRODUCTION_DAG_GAS_PORT_UNAVAILABLE")
    }

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

    fn distribute_rewards_with_native_plan(
        &self,
        request: &crate::FinalChainEvmRewardsRequest,
        plan: &crate::final_chain_execution::FinalChainPreparedExternalEvmRewardsStatsPlan,
    ) -> Result<crate::FinalChainEvmRewardsReport> {
        self.distribute_final_chain_rewards_with_native_plan(request, plan)
    }

    fn native_projection_context(
        &self,
        request_id: [u8; 32],
        projection_hash: [u8; 32],
    ) -> Result<Option<crate::native_projection_context::FinalChainNativeProjectionContext>> {
        self.final_chain_native_projection_context(request_id, projection_hash)
    }

    fn native_lifecycle_authority(
        &self,
        request_id: [u8; 32],
        projection_hash: [u8; 32],
    ) -> Result<Option<crate::native_projection_context::FinalChainLifecycleAuthority>> {
        self.final_chain_native_lifecycle_authority(request_id, projection_hash)
    }

    fn native_delayed_view(
        &self,
        request_id: [u8; 32],
        projection_hash: [u8; 32],
    ) -> Result<Option<std::rc::Rc<crate::native_session::frozen_eligibility::FrozenEligibility>>>
    {
        self.final_chain_native_delayed_view(request_id, projection_hash)
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
        while !{
            let __scope_arg_0_0 = self.final_chain;
            let _operation = self.runtime.composition.mutation()?;
            self.pbft.finalization_ready(__scope_arg_0_0)
        }? {
            match self.runtime.wait_for(
                self.generation,
                self.runtime.polling_interval_ms,
                self.process,
                None,
            )? {
                RuntimeWaitOutcome::Elapsed(_) => {}
                RuntimeWaitOutcome::Stopped => {
                    bail!("CONSENSUS_RUNTIME_STOPPED_DURING_FINALIZATION_WAIT")
                }
                RuntimeWaitOutcome::StaleWork => {
                    bail!("CONSENSUS_RUNTIME_FINALIZATION_WAIT_SOURCE_CHANGED")
                }
            }
        }
        Ok(())
    }

    fn report_malicious_peer(&self, peer_id: [u8; 64]) -> Result<()> {
        let id = self.runtime.next_effect(self.generation)?;
        let report = {
            let __scope_arg_0_0 = &ReportMaliciousPeerRequest {
                effect_id: id,
                peer_id,
                evidence_rlp: Vec::new(),
            };
            self.runtime
                .composition
                .callback()
                .and_then(|_operation| self.transport.report_malicious_peer(__scope_arg_0_0))
        }?;
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
    composition: ApplicationComposition,
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
    #[cfg(test)]
    pub(crate) fn exercise_bound_wait<P: ConsensusProcessPort>(
        &self,
        process: &P,
    ) -> Result<RuntimeWaitOutcome> {
        self.exercise_bound_wait_with_gap(process, || Ok(()))
    }

    #[cfg(test)]
    pub(crate) fn exercise_bound_wait_with_gap<P: ConsensusProcessPort>(
        &self,
        process: &P,
        gap: impl FnOnce() -> Result<()>,
    ) -> Result<RuntimeWaitOutcome> {
        let generation = self.begin_run()?;
        struct Reset<'a>(&'a ConsensusApplicationRuntime);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                if let Ok(_operation) = self.0.composition.mutation() {
                    self.0.running.store(false, Ordering::Release);
                }
            }
        }
        let _reset = Reset(self);
        let prepared_generation = self.composition.mutation()?.generation();
        gap()?;
        self.wait_for(generation, 1, process, prepared_generation)
    }

    pub(crate) fn in_composition(mut self, composition: ApplicationComposition) -> Self {
        self.composition = composition;
        self
    }

    pub(crate) fn is_bound_to_scope(
        &self,
        source: &std::sync::Arc<ApplicationScopeSource>,
    ) -> bool {
        self.composition.is_bound_to(source)
    }

    pub(crate) fn run_bound<P, S, T, E, O>(
        &self,
        services: ConsensusRuntimeServices<'_>,
        scope: &std::sync::Arc<ApplicationScopeSource>,
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
        ensure!(
            self.composition.is_bound_to(scope),
            "PRODUCTION_RUNTIME_SCOPE_MISMATCH"
        );
        self.run_inner(services, process, signer, transport, evm, observer)
    }

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
            composition: ApplicationComposition::Historical,
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
        let _operation = self.composition.mutation()?;
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
        let _operation = self.composition.mutation()?;
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
        let _operation = self.composition.mutation()?;
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
        let _operation = self.composition.mutation()?;
        self.next_effect_in_scope(generation)
    }

    fn next_effect_in_scope(&self, generation: u64) -> Result<ConsensusEffectId> {
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
        let _operation = self.composition.mutation()?;
        self.validate_report_in_scope(expected, actual)
    }

    fn validate_report_in_scope(
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
        prepared_generation: Option<u64>,
    ) -> Result<RuntimeWaitOutcome> {
        let ApplicationComposition::Production(source) = &self.composition else {
            let id = self.next_effect(generation)?;
            let report = process.wait(&ConsensusWaitRequest {
                effect_id: id,
                delay_ms,
            })?;
            self.validate_report(id, report.effect_id)?;
            return Ok(report.outcome.into());
        };
        let (request, expected_generation) = {
            let operation = source.mutation()?;
            if prepared_generation
                .is_some_and(|expected| expected.checked_add(1) != Some(operation.generation()))
            {
                return Ok(RuntimeWaitOutcome::StaleWork);
            }
            let id = self.next_effect(generation)?;
            (
                ConsensusWaitRequest {
                    effect_id: id,
                    delay_ms,
                },
                operation.generation(),
            )
        };
        let fence = source.wait_callback()?;
        if prepared_generation.is_some() && fence.generation() != expected_generation {
            return Ok(RuntimeWaitOutcome::StaleWork);
        }
        let result = process.wait(&request);
        drop(fence);
        let report = result?;
        let resumed = source.mutation()?;
        ensure!(
            self.running.load(Ordering::Acquire)
                && self.generation.load(Ordering::Acquire) == generation,
            "PRODUCTION_WAIT_RUNTIME_SESSION_CHANGED"
        );
        self.validate_report(request.effect_id, report.effect_id)?;
        if report.outcome == ConsensusWaitOutcome::Stopped {
            return Ok(RuntimeWaitOutcome::Stopped);
        }
        let resumed_generation = expected_generation
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("PRODUCTION_WAIT_GENERATION_EXHAUSTED"))?;
        if prepared_generation.is_some() && resumed.generation() != resumed_generation {
            return Ok(RuntimeWaitOutcome::StaleWork);
        }
        Ok(RuntimeWaitOutcome::Elapsed(Some(resumed.generation())))
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
        self.composition
            .require_execution_plan("private signing plan")?;
        ensure!(
            self.signing_identities
                .get(wallet_index as usize)
                .is_some_and(|identity| identity.wallet_index == wallet_index),
            "CONSENSUS_RUNTIME_SIGNER_UNKNOWN"
        );
        let id = self.next_effect(generation)?;
        let report = {
            let __scope_arg_0_0 = &ConsensusSignRequest {
                effect_id: id,
                wallet_index,
                digest,
            };
            self.composition
                .callback()
                .and_then(|_operation| signer.sign_digest(__scope_arg_0_0))
        }?;
        self.validate_report(id, report.effect_id)?;
        Self::checked_signature(report)
    }

    fn checked_signature(report: ConsensusSignReport) -> Result<Vec<u8>> {
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
        let report = {
            let __scope_arg_0_0 = &ConsensusVrfRequest {
                effect_id: id,
                wallet_index,
                message,
            };
            self.composition
                .callback()
                .and_then(|_operation| signer.prove_vrf(__scope_arg_0_0))
        }?;
        self.validate_report(id, report.effect_id)?;
        Self::checked_vrf_report(report)
    }

    fn checked_vrf_report(report: ConsensusVrfReport) -> Result<Vec<u8>> {
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
        let report = {
            let __scope_arg_0_0 = &GossipVoteRequest {
                effect_id: id,
                vote_rlp,
                proposed_block_rlp,
                rebroadcast,
            };
            self.composition
                .callback()
                .and_then(|_operation| transport.gossip_vote(__scope_arg_0_0))
        }?;
        self.validate_report(id, report.effect_id)?;
        Ok(report)
    }

    fn sign_bound_pbft_vote<S: ConsensusSigningPort>(
        &self,
        generation: u64,
        final_chain: &FinalChain,
        pbft: &PbftService,
        vrf: PbftVoteVrfRequest,
        signer: &S,
    ) -> Result<Option<PbftGeneratedVote>> {
        let wallet_index = vrf.input.wallet_index;
        let effect_id = self.next_effect_in_scope(generation)?;
        let report = signer.prove_vrf(&ConsensusVrfRequest {
            effect_id,
            wallet_index,
            message: vrf.message.clone(),
        })?;
        self.validate_report_in_scope(effect_id, report.effect_id)?;
        let proof = Self::checked_vrf_report(report)?;
        let Some(signing) = pbft.prepare_state_vote_signing_inner(final_chain, vrf, proof)? else {
            return Ok(None);
        };
        let effect_id = self.next_effect_in_scope(generation)?;
        let report = signer.sign_digest(&ConsensusSignRequest {
            effect_id,
            wallet_index,
            digest: signing.signing_hash.0,
        })?;
        self.validate_report_in_scope(effect_id, report.effect_id)?;
        Ok(Some(complete_pbft_vote_signing(
            signing,
            Self::checked_signature(report)?,
        )?))
    }

    #[allow(clippy::too_many_arguments)]
    fn run_bound_state_vote<
        S: ConsensusSigningPort,
        T: ConsensusTransportPort,
        E: ConsensusExecutionPort,
    >(
        &self,
        generation: u64,
        mut action_generation: u64,
        state: PbftManagerRuntimeStateCode,
        request: ConsensusStateActionRequest,
        services: ConsensusRuntimeServices<'_>,
        signer: &S,
        transport: &T,
        execution: &E,
    ) -> Result<Option<(bool, bool)>> {
        let pbft = services.pbft;
        let (initial, batch) = {
            let operation = self.composition.callback()?;
            if operation.generation() != Some(action_generation) {
                return Ok(None);
            }
            let manager = pbft.manager_snapshot();
            ensure!(
                manager.state == state,
                "PRODUCTION_BOUND_VOTE_CURSOR_MISMATCH"
            );
            (
                manager,
                compose_bound_state_vote(pbft, services.final_chain, services.dag, request)?,
            )
        };
        let flags = Some((batch.go_finish_state, batch.loop_back_finish_state));
        let task_limit = if state == PbftManagerRuntimeStateCode::FinishPolling {
            2
        } else {
            1
        };
        ensure!(
            batch.votes.len() <= task_limit,
            "PRODUCTION_BOUND_VOTE_TASK_COUNT"
        );
        for (index, task) in batch.votes.iter().enumerate() {
            let manager = if index == 0 {
                initial.clone()
            } else {
                let operation = self.composition.callback()?;
                if operation.generation() != Some(action_generation) {
                    return Ok(None);
                }
                let observed = pbft.manager_snapshot();
                if (
                    observed.state,
                    observed.period,
                    observed.round,
                    observed.step,
                ) != (initial.state, initial.period, initial.round, initial.step)
                    || !compose_bound_state_vote(pbft, services.final_chain, services.dag, request)?
                        .votes
                        .contains(task)
                {
                    return Ok(None);
                }
                observed
            };
            let Some(next) = self.run_bound_state_vote_task(
                generation,
                action_generation,
                &manager,
                task,
                &services,
                signer,
                transport,
                execution,
            )?
            else {
                return Ok(None);
            };
            action_generation = next;
        }
        Ok(flags)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_bound_state_vote_task<
        S: ConsensusSigningPort,
        T: ConsensusTransportPort,
        E: ConsensusExecutionPort,
    >(
        &self,
        generation: u64,
        action_generation: u64,
        manager: &crate::pbft_manager::PbftManagerRuntimeSnapshot,
        task: &ConsensusStateVoteTask,
        services: &ConsensusRuntimeServices<'_>,
        signer: &S,
        transport: &T,
        execution: &E,
    ) -> Result<Option<u64>> {
        let ApplicationComposition::Production(source) = &self.composition else {
            bail!("PRODUCTION_BOUND_VOTE_OWNER_REQUIRED");
        };
        let pbft = services.pbft;
        let final_chain = services.final_chain;
        use ConsensusStateVoteCommit::{MarkNextVotedNull, MarkNextVotedValue, SaveCertVotedBlock};
        use PbftManagerRuntimeStateCode::{Certify, Filter, Finish, FinishPolling};
        let null = task.block_hash.is_zero();
        let mode = match (manager.state, task.vote_type, task.commit, null) {
            (Filter, PbftVoteType::Soft, ConsensusStateVoteCommit::None, false) => {
                task.step == PbftVoteType::Soft as u64
            }
            (Certify, PbftVoteType::Cert, SaveCertVotedBlock, false) => {
                task.step == PbftVoteType::Cert as u64
            }
            (Finish, PbftVoteType::Next, ConsensusStateVoteCommit::None, _) => {
                task.step >= 4 && task.step.is_multiple_of(2)
            }
            (FinishPolling, PbftVoteType::Next, MarkNextVotedNull, true)
            | (FinishPolling, PbftVoteType::Next, MarkNextVotedValue, false) => {
                task.step >= 5 && !task.step.is_multiple_of(2)
            }
            _ => false,
        };
        ensure!(
            mode && task.period == manager.period
                && task.round == manager.round
                && task.step == manager.step,
            "PRODUCTION_BOUND_VOTE_TASK_MISMATCH"
        );
        let proof = if null {
            ensure!(
                task.proposed_block_rlp.is_empty(),
                "PRODUCTION_NULL_VOTE_HAS_CANDIDATE"
            );
            None
        } else {
            let link = rustaxa_types::pbft::PbftBlockLink::try_from(
                rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp::new(&task.proposed_block_rlp),
            )?;
            ensure!(
                link.block_hash == task.block_hash && link.period == task.period,
                "PRODUCTION_BOUND_VOTE_TASK_MISMATCH"
            );
            match execution
                .check_proposal_transactions(&task.proposed_block_rlp, action_generation)?
            {
                None => return Ok(None),
                Some(ProposalExecutionCheck::Invalid) => return Ok(Some(action_generation)),
                Some(ProposalExecutionCheck::Unavailable(reason)) => {
                    bail!("PRODUCTION_BOUND_VOTE_EXECUTION_UNAVAILABLE: {reason:?}");
                }
                Some(ProposalExecutionCheck::Checked(proof)) => Some(proof),
            }
        };
        let current = |operation: &crate::consensus_application::production_application::ApplicationOperationScope<'_>, writing| {
            if let Some(proof) = &proof {
                source.check_proposal_vote(proof, operation, &task.proposed_block_rlp, task.period, writing)
            } else {
                Ok(if writing {
                    operation.follows(Some(action_generation))
                } else {
                    operation.generation()==Some(action_generation)
                })
            }
        };
        let split_slots = task.vote_type == PbftVoteType::Next && !task.step.is_multiple_of(2);
        let (signed, conflicted) = {
            let operation = self.composition.callback()?;
            if pbft.manager_snapshot() != *manager || !current(&operation, false)? {
                return Ok(None);
            }
            let configured_voters: std::collections::HashSet<_> = self
                .signing_identities
                .iter()
                .map(|identity| identity.address)
                .collect();
            ensure!(
                configured_voters.len() == self.signing_identities.len(),
                "PRODUCTION_BOUND_VOTE_DUPLICATE_WALLET"
            );
            let mut current_voters = std::collections::HashMap::new();
            for stored in pbft.verified_votes_own_vote_records()? {
                let vote = inspect_canonical_pbft_vote(&stored.vote_rlp)?;
                ensure!(
                    vote.signature_valid && vote.vote_hash == stored.hash,
                    "PRODUCTION_BOUND_VOTE_STORED_IDENTITY_MISMATCH"
                );
                if vote.period == task.period && vote.round == task.round && vote.step == task.step
                {
                    let key = (
                        vote.recovered_voter,
                        split_slots && vote.block_hash.is_zero(),
                    );
                    ensure!(
                        task.vote_type != PbftVoteType::Next
                            || vote.vote_type == PbftVoteType::Next,
                        "PRODUCTION_BOUND_VOTE_STORED_TYPE_MISMATCH"
                    );
                    ensure!(
                        current_voters
                            .insert(key, (vote, stored.vote_rlp))
                            .is_none(),
                        "PRODUCTION_BOUND_VOTE_DUPLICATE_STORED_SLOT"
                    );
                }
            }
            if task.commit == SaveCertVotedBlock
                && self.signing_identities.iter().any(|identity| {
                    current_voters
                        .get(&(ethereum_types::H160(identity.address), false))
                        .is_some_and(|(vote, _)| {
                            vote.block_hash != task.block_hash
                                || vote.vote_type != task.vote_type
                                || vote.recovered_public_key != identity.node_public_key
                        })
                })
            {
                return Ok(Some(action_generation));
            }
            let mut signed = Vec::new();
            let mut conflicted = false;
            for identity in &self.signing_identities {
                let voter = ethereum_types::H160(identity.address);
                if let Some((vote, vote_rlp)) = current_voters.get(&(voter, split_slots && null)) {
                    let matches = vote.block_hash == task.block_hash
                        && vote.vote_type == task.vote_type
                        && vote.recovered_public_key == identity.node_public_key;
                    if matches {
                        signed.push(BoundStateVote::Retained(vote_rlp.clone()));
                    }
                    conflicted |= !matches;
                    continue;
                }
                let vrf = prepare_pbft_vote_vrf(PbftVoteGenerationPublicInput {
                    wallet_index: identity.wallet_index,
                    block_hash: task.block_hash,
                    vote_type: task.vote_type,
                    period: task.period,
                    round: task.round,
                    step: task.step,
                    voter,
                    voter_public_key: identity.node_public_key,
                    vrf_public_key: identity.vrf_public_key,
                })?;
                if let Some(vote) =
                    self.sign_bound_pbft_vote(generation, final_chain, pbft, vrf, signer)?
                {
                    signed.push(BoundStateVote::Signed(vote));
                }
            }
            (signed, conflicted)
        };
        if signed.is_empty() && (conflicted || task.commit == ConsensusStateVoteCommit::None) {
            return Ok(Some(action_generation));
        }
        let (admitted_generation, egress) = {
            let operation = self.composition.mutation()?;
            if pbft.manager_snapshot() != *manager || !current(&operation, true)? {
                return Ok(None);
            }
            let mut egress = Vec::new();
            for vote in signed {
                let vote_rlp = match vote {
                    BoundStateVote::Retained(vote_rlp) => vote_rlp,
                    BoundStateVote::Signed(vote) => {
                        let admission = pbft.admit_state_vote_with_slashing_resolver(
                            final_chain,
                            task,
                            &vote,
                            || bail!("PRODUCTION_BOUND_VOTE_SLASHING_UNAVAILABLE"),
                        )?;
                        ensure!(
                            admission.transaction.transition_published,
                            "PRODUCTION_BOUND_VOTE_ADMISSION_NOT_PUBLISHED: {}",
                            admission.transaction.persistence_error_code
                        );
                        if !admission.validation.accepted {
                            continue;
                        }
                        vote.vote_rlp
                    }
                };
                egress.push(ProposalEgressPacket::Vote(GossipVoteRequest {
                    effect_id: self.next_effect_in_scope(generation)?,
                    vote_rlp,
                    proposed_block_rlp: task.proposed_block_rlp.clone(),
                    rebroadcast: false,
                }));
            }
            let admitted_generation = operation
                .generation()
                .context("PRODUCTION_BOUND_VOTE_GENERATION_MISSING")?;
            (admitted_generation, egress)
        };
        let sent =
            self.send_persisted_proposal(admitted_generation, egress, pbft, transport, false)?;
        let current_generation = match sent {
            OwnedVoteTransport::Completed(generation) => generation,
            OwnedVoteTransport::Stale | OwnedVoteTransport::Incomplete => return Ok(None),
        };
        if task.commit == ConsensusStateVoteCommit::None {
            return Ok(Some(current_generation));
        }
        let operation = self.composition.mutation()?;
        if !operation.follows(Some(current_generation)) || pbft.manager_snapshot() != *manager {
            return Ok(None);
        }
        pbft.commit_state_vote_task(task)?;
        Ok(operation.generation())
    }

    fn send_persisted_proposal<T: ConsensusTransportPort>(
        &self,
        mut generation: u64,
        packets: Vec<ProposalEgressPacket>,
        pbft: &PbftService,
        transport: &T,
        require_delivery: bool,
    ) -> Result<OwnedVoteTransport> {
        let ApplicationComposition::Production(source) = &self.composition else {
            bail!("PRODUCTION_PROPOSAL_OWNER_MISSING");
        };
        for packet in packets {
            let request_id = packet.effect_id();
            let Some(lease) = source.begin_proposal_egress(generation, packet.clone())? else {
                return Ok(OwnedVoteTransport::Stale);
            };
            let is_rewards = matches!(packet, ProposalEgressPacket::VoteBundle(_));
            let report = packet.send(transport);
            let completion = lease.finish()?;
            if let Some(token) = completion.pending {
                let _operation = self.composition.mutation()?;
                pbft.network_service().cancel_egress(token)?;
            }
            if let Some(lane) = completion.abandoned_lane {
                let _operation = self.composition.mutation()?;
                pbft.network_service()
                    .cancel_proposal_egress_effects(lane, request_id.sequence)?;
            }
            let report = report?;
            self.validate_report_in_scope(request_id, report.effect_id)?;
            if completion.stale {
                return Ok(OwnedVoteTransport::Stale);
            }
            if require_delivery && !report.succeeded {
                return Ok(OwnedVoteTransport::Incomplete);
            }
            ensure!(
                !is_rewards || report.succeeded,
                "CONSENSUS_PROPOSAL_REWARD_GOSSIP_FAILED: {}",
                report.error_code
            );
            if !completion.complete {
                if !completion.started
                    && !report.succeeded
                    && report.error_code == "TRANSPORT_UNAVAILABLE"
                {
                    let operation = self.composition.callback()?;
                    if operation.generation() != Some(generation) {
                        return Ok(OwnedVoteTransport::Stale);
                    }
                    continue;
                }
                ensure!(!report.succeeded, "PRODUCTION_PROPOSAL_EGRESS_INCOMPLETE");
                return Ok(OwnedVoteTransport::Incomplete);
            }
            generation = completion.generation;
        }
        Ok(OwnedVoteTransport::Completed(generation))
    }

    pub(crate) fn proposal_period_dag_level_update(
        &self,
        anchor_rlp: &[u8],
    ) -> Result<crate::FinalChainProposalPeriodDagLevelUpdate> {
        if anchor_rlp.is_empty() {
            return Ok(crate::FinalChainProposalPeriodDagLevelUpdate::default());
        }
        let anchor = rustaxa_types::DagBlock::try_from(
            rustaxa_types::codec::rlp::dag::DagBlockRlp::new(anchor_rlp),
        )?;
        Ok(crate::FinalChainProposalPeriodDagLevelUpdate {
            has_update: true,
            level: anchor
                .level
                .checked_add(self.max_levels_per_period)
                .context("FINAL_CHAIN_PROPOSAL_PERIOD_LEVEL_OVERFLOW")?,
        })
    }

    pub(crate) fn execute_final_chain_task<E: ConsensusExecutionPort>(
        &self,
        pbft: &PbftService,
        final_chain: &FinalChain,
        request: EvmFinalizationRequest,
        evm: &E,
    ) -> Result<crate::FinalChainApplicationExecutionReport> {
        self.composition
            .require_execution_plan("period execution and publication")?;
        let execution_request =
            crate::pbft_application_finalization::final_chain_execution_request_from_period_data(
                &request.period_data_rlp,
                &request.previous_cert_vote_rlps,
                &request.previous_cert_vote_weights,
                request.blocks_per_year,
                {
                    let _operation = self.composition.callback()?;
                    final_chain.block_gas_limit()
                },
            )?;
        let period = rustaxa_types::PbftBlockMetadata::try_from(
            rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp::new(
                &execution_request.pbft_block_rlp,
            ),
        )?
        .period;
        let period: rustaxa_types::FinalChainBlockNumber = period.into();
        let expected_next = {
            let _operation = self.composition.mutation()?;
            final_chain.committed_state_descriptor()
        }?
        .period
        .checked_next()
        .context("FINAL_CHAIN_RUNTIME_PRIOR_PERIOD_OVERFLOW")?;
        ensure!(
            period == expected_next,
            "FINAL_CHAIN_RUNTIME_NON_CONSECUTIVE_PERIOD: expected {}, requested {}",
            expected_next.as_u64(),
            period.as_u64()
        );
        {
            let __scope_arg_0_1 = &request.period_data_rlp;
            let __scope_arg_0_0 = period;
            let _operation = self.composition.mutation()?;
            final_chain.ensure_period_data(__scope_arg_0_0, __scope_arg_0_1)
        }?;
        let proposal_period_update =
            self.proposal_period_dag_level_update(&request.anchor_block_rlp)?;
        let (ficus_activation, pillar_interval) = {
            let _operation = self.composition.callback()?;
            pbft.pillar_schedule()
        };
        let system_period = period
            .as_u64()
            .checked_add({
                let _operation = self.composition.callback()?;
                final_chain.dpos_delegation_delay()
            })
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
        let observed_block = {
            let _operation = self.composition.mutation()?;
            final_chain.last_block_number_typed()
        }?;
        if let ApplicationComposition::Production(scope) = &self.composition {
            let records = scope.account_records(observed_block, &addresses)?;
            let accounts = addresses
                .into_iter()
                .zip(records)
                .map(|(address, record)| match record {
                    Some(row) => NativeFinalChainAccountFact {
                        address,
                        found: true,
                        nonce: row.account.nonce.to_bytes(),
                        balance: row.account.balance.value().to_bytes_be(),
                    },
                    None => NativeFinalChainAccountFact {
                        address,
                        found: false,
                        nonce: Vec::new(),
                        balance: Vec::new(),
                    },
                })
                .collect();
            return Ok(NativeFinalChainAccountFacts {
                observed_block: observed_block.as_u64(),
                accounts,
            });
        }
        let accounts = addresses
            .into_iter()
            .map(|address| {
                let account = {
                    let __scope_arg_0_1 = address;
                    let __scope_arg_0_0 = observed_block;
                    let _operation = self.composition.mutation()?;
                    final_chain.account_at_block(__scope_arg_0_0, __scope_arg_0_1)
                }?;
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

        let block_header_rlp = {
            let __scope_arg_0_0 = period.into();
            let _operation = self.composition.mutation()?;
            final_chain.block_header(__scope_arg_0_0)
        }?
        .context("PILLAR_ANCHOR_HEADER_MISSING")?;
        let state_root = Rlp::new(&block_header_rlp)
            .val_at::<ethereum_types::H256>(3)
            .context("PILLAR_ANCHOR_STATE_ROOT_DECODE")?;
        report.block_header_rlp = block_header_rlp;
        report.state_root = state_root.into();
        report.validator_vote_counts = {
            let __scope_arg_0_0 = pillar_block_period.into();
            let _operation = self.composition.mutation()?;
            final_chain.dpos_validators_eligible_vote_counts(__scope_arg_0_0)
        }?
        .into_iter()
        .map(|count| PillarAnchorValidatorVoteCount {
            address: count.address,
            vote_count: count.vote_count,
        })
        .collect();
        report.signer_vote_counts = signer_addresses
            .into_iter()
            .map(|address| {
                let __scope_arg_0_1 = address;
                let __scope_arg_0_0 = pillar_block_period.into();
                let _operation = self.composition.mutation()?;
                final_chain.dpos_eligible_vote_count(__scope_arg_0_0, __scope_arg_0_1)
            })
            .collect::<Result<Vec<_>>>()?;
        report.total_eligible_vote_count = {
            let __scope_arg_0_0 = pillar_block_period.into();
            let _operation = self.composition.mutation()?;
            final_chain.dpos_eligible_total_vote_count(__scope_arg_0_0)
        }?;
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
        if let Ok(report) = {
            let __scope_arg_0_0 = &ConsensusObservationRequest {
                effect_id,
                kind: CONSENSUS_OBSERVATION_KIND_PILLAR_BLOCK,
                period: 0,
                hash: observation.block_hash,
                canonical_rlp: observation.block_data_rlp.clone(),
            };
            self.composition
                .callback()
                .and_then(|_operation| observer.observe(__scope_arg_0_0))
        } {
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
        self.observe_finalized_block_identity(
            generation,
            report.period.as_u64(),
            report.block_hash,
            observer,
        );
    }

    fn observe_finalized_block_identity<O: ConsensusObserverPort>(
        &self,
        generation: u64,
        period: u64,
        block_hash: [u8; 32],
        observer: &O,
    ) {
        let Ok(effect_id) = self.next_effect(generation) else {
            return;
        };
        if let Ok(ack) = {
            let __scope_arg_0_0 = &ConsensusObservationRequest {
                effect_id,
                kind: CONSENSUS_OBSERVATION_KIND_FINALIZED_BLOCK,
                period,
                hash: block_hash,
                canonical_rlp: Vec::new(),
            };
            self.composition
                .callback()
                .and_then(|_operation| observer.observe(__scope_arg_0_0))
        } {
            let _ = self.validate_report(effect_id, ack.effect_id);
        }
    }

    fn observe_production_publication<O: ConsensusObserverPort>(
        &self,
        generation: u64,
        period: u64,
        pbft: &PbftService,
        final_chain: &FinalChain,
        observer: &O,
    ) {
        let durable = || -> Result<_> {
            let _operation = self.composition.callback()?;
            let hash = final_chain
                .block_hash(period.into())?
                .context("PRODUCTION_OBSERVATION_BLOCK_HASH_MISSING")?;
            let hash: [u8; 32] = hash.as_slice().try_into()?;
            Ok((
                hash,
                period
                    .checked_sub(1)
                    .and_then(|prior| pbft.finalized_pillar_observation(prior).ok().flatten()),
            ))
        };
        if let Ok((hash, pillar)) = durable() {
            self.observe_finalized_pillar(generation, pillar.as_ref(), observer);
            self.observe_finalized_block_identity(generation, period, hash, observer);
        }
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
        let vrf_public_key = input.wallet_vrf_public_key;
        let (session_id, mut owner_generation) = {
            let scope = self.composition.mutation()?;
            if let ApplicationComposition::Production(source) = &self.composition {
                ensure!(
                    self.signing_identities
                        .get(wallet_index as usize)
                        .is_some_and(|identity| {
                            identity.wallet_index == wallet_index
                                && identity.address == input.proposer_address
                                && identity.vrf_public_key == input.wallet_vrf_public_key
                        }),
                    "PRODUCTION_DAG_PROPOSER_WALLET_MISMATCH"
                );
                let prior = final_chain.last_block_number_typed()?;
                if prior.checked_add_distance(1).map(|next| next.as_u64())
                    != Some(pbft.manager_snapshot().period)
                {
                    return Ok(false);
                }
                let period = pbft.manager_snapshot().period;
                let votes = final_chain
                    .pbft_dpos_eligible_wallet_vote_counts(period, &[input.proposer_address])?
                    .context("PRODUCTION_PROPOSER_ELIGIBILITY_UNAVAILABLE")?;
                ensure!(votes.len() == 1, "PRODUCTION_PROPOSER_ELIGIBILITY_COUNT");
                if votes[0].vote_count == 0 {
                    return Ok(false);
                }
                source.account_records(prior, &[])?;
            }
            (dag.begin_proposer_session(input)?, scope.generation())
        };
        let mutation = |generation: &mut Option<u64>| -> Result<_> {
            let scope = self.composition.mutation()?;
            if !scope.follows(*generation) {
                return Err(DagProposerStale.into());
            }
            *generation = scope.generation();
            Ok(scope)
        };
        let callback = |generation: Option<u64>| -> Result<_> {
            let scope = self.composition.callback()?;
            if scope.generation() != generation {
                return Err(DagProposerStale.into());
            }
            Ok(scope)
        };
        let mut active_vdf_job = None;
        let mut active_add = None;
        let result: Result<bool> = (|| {
            let mut step = {
                let __scope_arg_0_1 = final_chain;
                let __scope_arg_0_0 = session_id;
                let _operation = mutation(&mut owner_generation)?;
                dag.report_proposer_final_chain_facts_with_final_chain(
                    __scope_arg_0_0,
                    __scope_arg_0_1,
                )
            }?;
            if matches!(step.action, DagProposerSessionAction::Complete) {
                return Ok(step.return_value);
            }
            ensure!(
                matches!(step.action, DagProposerSessionAction::ProveVrf),
                "CONSENSUS_DAG_PROPOSER_EXPECTED_VRF"
            );
            let vrf_id = {
                let _scope = mutation(&mut owner_generation)?;
                self.next_effect_in_scope(generation)?
            };
            let vrf = {
                let __scope_arg_0_0 = &ConsensusVrfRequest {
                    effect_id: vrf_id,
                    wallet_index,
                    message: step.vrf_input.clone(),
                };
                let _scope = callback(owner_generation)?;
                signer.prove_vrf(__scope_arg_0_0)
            }?;
            self.validate_report_in_scope(vrf_id, vrf.effect_id)?;
            ensure!(
                vrf.succeeded,
                "CONSENSUS_RUNTIME_VRF_FAILED: {}",
                vrf.error_code
            );
            step = {
                let __scope_arg_0_1 = DagProposerVrfReport {
                    proof: vrf.proof,
                    output: vrf.output,
                };
                let __scope_arg_0_0 = session_id;
                let _operation = mutation(&mut owner_generation)?;
                dag.report_proposer_vrf(__scope_arg_0_0, __scope_arg_0_1)
            }?;
            if matches!(step.action, DagProposerSessionAction::Complete) {
                return Ok(step.return_value);
            }
            ensure!(
                matches!(step.action, DagProposerSessionAction::PackTransactions),
                "CONSENSUS_DAG_PROPOSER_EXPECTED_PACK"
            );
            let (dag_gas_limit, pbft_gas_limit) =
                self.dag_proposer_config.gas_limits(step.proposal_period);
            {
                let __scope_arg_0_3 = dag_gas_limit;
                let __scope_arg_0_2 = pbft_gas_limit;
                let __scope_arg_0_1 = self
                    .dag_proposer_config
                    .proposal_weight_limit(step.proposal_period);
                let __scope_arg_0_0 = session_id;
                let _operation = mutation(&mut owner_generation)?;
                dag.configure_proposer_gas_policy(
                    __scope_arg_0_0,
                    __scope_arg_0_1,
                    __scope_arg_0_2,
                    __scope_arg_0_3,
                )
            }?;
            let (pack, committed_period) = {
                let status = {
                    let _operation = callback(owner_generation)?;
                    transport.transport_status()
                };
                let now = {
                    let _operation = callback(owner_generation)?;
                    process.now_millis()
                };
                let _scope = mutation(&mut owner_generation)?;
                let committed_period = final_chain.last_block_number()?;
                let request = DagProposerPackPrepareRequest {
                    session_id,
                    network_throttled: pbft.network_service().pbft_sync_status(now)?.active
                        || status.packet_queue_over_limit,
                    min_transaction_gas: 21_000,
                    estimate_gas_limit: 200_000,
                    last_block_number: committed_period,
                };
                (dag.prepare_proposer_pack(request)?, committed_period)
            };
            step = pack.session;
            if !pack.estimate_requests.is_empty() {
                let (effect_id, estimate_generation) = {
                    let _scope = mutation(&mut owner_generation)?;
                    (self.next_effect_in_scope(generation)?, owner_generation)
                };
                let request = DagGasEstimateRequest {
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
                };
                let (response_id, estimates) = match &self.composition {
                    ApplicationComposition::Production(source) => {
                        let Some(report) =
                            execution.estimate_owned_dag_transaction_gas(OwnedDagGasRequest {
                                source,
                                generation: estimate_generation
                                    .context("PRODUCTION_DAG_GAS_GENERATION_MISSING")?,
                                committed_period,
                                origin: OwnedDagGasOrigin::Pack(session_id),
                                request: &request,
                            })?
                        else {
                            return Err(DagProposerStale.into());
                        };
                        let estimates = report
                            .estimates
                            .into_iter()
                            .map(|estimate| TransactionPackingEstimate {
                                hash: estimate.hash.into(),
                                gas_used: estimate.gas_used,
                                last_block_number: committed_period,
                                result_rlp: Vec::new(),
                            })
                            .collect::<Vec<_>>();
                        (report.effect_id, estimates)
                    }
                    ApplicationComposition::Historical => {
                        let report = execution.estimate_dag_transaction_gas(&request)?;
                        self.validate_report_in_scope(effect_id, report.effect_id)?;
                        ensure!(
                            report.succeeded,
                            "CONSENSUS_DAG_GAS_FAILED: {}",
                            report.error_code
                        );
                        let estimates = report
                            .estimates
                            .into_iter()
                            .map(|estimate| TransactionPackingEstimate {
                                hash: estimate.hash.into(),
                                gas_used: estimate.gas_used,
                                last_block_number: report.observed_block,
                                result_rlp: estimate.result_rlp,
                            })
                            .collect();
                        (report.effect_id, estimates)
                    }
                };
                ensure!(
                    estimates.len() == pack.estimate_requests.len(),
                    "CONSENSUS_DAG_GAS_COUNT_MISMATCH"
                );
                let _scope = mutation(&mut owner_generation)?;
                self.validate_report_in_scope(effect_id, response_id)?;
                step = dag.finalize_proposer_pack(session_id, estimates)?.session;
            }
            if matches!(step.action, DagProposerSessionAction::Complete) {
                return Ok(step.return_value);
            }
            ensure!(
                matches!(step.action, DagProposerSessionAction::StartVdf),
                "CONSENSUS_DAG_PROPOSER_EXPECTED_VDF"
            );
            let job_id = {
                let __scope_arg_0_0 = NativeVdfRequest {
                    vrf_proof: step.vrf_proof.clone(),
                    vdf_message: step.vdf_message.clone(),
                    lambda_bound: step.sortition_params.vdf.lambda_bound,
                    difficulty: step.vdf_difficulty,
                };
                let _operation = mutation(&mut owner_generation)?;
                self.vdf_executor.start(__scope_arg_0_0)
            }?;
            active_vdf_job = Some(job_id);
            let vdf_rlp = loop {
                let native = {
                    let __scope_arg_0_0 = session_id;
                    let _operation = mutation(&mut owner_generation)?;
                    dag.poll_proposer_vdf(__scope_arg_0_0)
                }?;
                if matches!(native.action, DagProposerSessionAction::CancelVdf) || {
                    let __scope_arg_0_0 = generation;
                    let _operation = callback(owner_generation)?;
                    process.stop_requested(__scope_arg_0_0)
                } {
                    let _scope = mutation(&mut owner_generation)?;
                    active_vdf_job = None;
                    self.vdf_executor.cancel(job_id)?;
                    dag.abort_proposer_session(session_id)?;
                    return Ok(native.return_value);
                }
                match {
                    let __scope_arg_0_0 = job_id;
                    let _operation = mutation(&mut owner_generation)?;
                    self.vdf_executor.poll(__scope_arg_0_0)
                }? {
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
                match self.wait_for(
                    generation,
                    crate::dag::DAG_PROPOSER_VDF_POLL_INTERVAL_MS,
                    process,
                    None,
                )? {
                    RuntimeWaitOutcome::Elapsed(resumed) => owner_generation = resumed,
                    RuntimeWaitOutcome::Stopped | RuntimeWaitOutcome::StaleWork => {
                        return Err(DagProposerStale.into());
                    }
                }
            };
            step = {
                if owner_generation.is_some() {
                    use rustaxa_vdf::sortition::{
                        LEGACY_SORTITION_STATUS_VALID, LegacySortitionParams,
                        decode_legacy_vdf_sortition, verify_legacy_vdf_sortition,
                    };
                    let proof = decode_legacy_vdf_sortition(&vdf_rlp)?;
                    ensure!(
                        proof.vrf_proof.as_slice() == step.vrf_proof.as_slice()
                            && proof.difficulty == step.vdf_difficulty,
                        "PRODUCTION_DAG_VDF_RESULT_MISMATCH"
                    );
                    let params = step.sortition_params;
                    let verified = verify_legacy_vdf_sortition(
                        LegacySortitionParams {
                            vrf_threshold_upper: params.vrf.threshold_upper,
                            vdf_difficulty_min: params.vdf.difficulty_min,
                            vdf_difficulty_max: params.vdf.difficulty_max,
                            vdf_difficulty_stale: params.vdf.difficulty_stale,
                            vdf_lambda_bound: params.vdf.lambda_bound,
                        },
                        &vrf_public_key,
                        &vdf_rlp,
                        &step.vrf_input,
                        &step.vdf_message,
                        step.vote_count,
                        step.max_vote_count,
                    )?;
                    ensure!(
                        verified.status == LEGACY_SORTITION_STATUS_VALID,
                        "PRODUCTION_DAG_VDF_VERIFICATION_FAILED: {}",
                        verified.status
                    );
                }
                let __scope_arg_0_1 = DagProposerVdfProofReport {
                    proof_ok: true,
                    vdf_rlp,
                };
                let __scope_arg_0_0 = session_id;
                let _operation = mutation(&mut owner_generation)?;
                dag.report_proposer_vdf_proof(__scope_arg_0_0, __scope_arg_0_1)
            }?;
            if matches!(step.action, DagProposerSessionAction::StaleProofSleep) {
                match self.wait_for(
                    generation,
                    crate::dag::DAG_PROPOSER_STALE_PROOF_SLEEP_MS,
                    process,
                    None,
                )? {
                    RuntimeWaitOutcome::Elapsed(resumed) => owner_generation = resumed,
                    RuntimeWaitOutcome::Stopped | RuntimeWaitOutcome::StaleWork => {
                        return Err(DagProposerStale.into());
                    }
                }
                step = {
                    let __scope_arg_0_0 = session_id;
                    let _operation = mutation(&mut owner_generation)?;
                    dag.resume_proposer_stale_proof(__scope_arg_0_0)
                }?;
            }
            if matches!(step.action, DagProposerSessionAction::Complete) {
                return Ok(step.return_value);
            }
            ensure!(
                matches!(step.action, DagProposerSessionAction::SignBlock),
                "CONSENSUS_DAG_PROPOSER_EXPECTED_SIGN"
            );
            let signing_effect = {
                let scope = mutation(&mut owner_generation)?;
                // DAG proof waits allow network work to proceed. The proof
                // stages check the frontier again before this admission check.
                if scope.generation().is_some()
                    && (final_chain.last_block_number()? != committed_period
                        || !dag.transactions_are_proposable(&step.selected_transaction_hashes)?)
                {
                    return Err(DagProposerStale.into());
                }
                self.next_effect_in_scope(generation)?
            };
            let signature = {
                let _scope = callback(owner_generation)?;
                let report = signer.sign_digest(&ConsensusSignRequest {
                    effect_id: signing_effect,
                    wallet_index,
                    digest: step.signing_hash.0,
                })?;
                self.validate_report_in_scope(signing_effect, report.effect_id)?;
                Self::checked_signature(report)?
            };
            step = {
                let __scope_arg_0_1 = DagProposerSigningReport { signature };
                let __scope_arg_0_0 = session_id;
                let _operation = mutation(&mut owner_generation)?;
                dag.report_proposer_signing(__scope_arg_0_0, __scope_arg_0_1)
            }?;
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
            let (emit_verified, gossip) = {
                let _scope = mutation(&mut owner_generation)?;
                let prepared = dag.prepare_add_block(DagAddBlockPrepareRequest {
                    expected_hash: signed.block_hash,
                    block_rlp: signed.block_rlp.clone(),
                    validate_hash: true,
                    save: true,
                    proposed: true,
                    transactions,
                })?;
                let (add, emit_verified, gossip) = if prepared.cursor_id == 0 {
                    (
                        DagProposerAddBlockReport {
                            accepted: prepared.accepted,
                            duplicate: prepared.duplicate,
                            expired: prepared.expired,
                            missing_references: prepared.missing_references,
                        },
                        false,
                        false,
                    )
                } else {
                    active_add = Some(prepared.cursor_id);
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
                            .map(|(idx, value)| {
                                use crate::consensus_application::production_application::bounded_account_integer;
                                Ok(DagAddBlockAccountNonceFact {
                                    input_index: idx as u64,
                                    account_nonce: bounded_account_integer(&value.nonce)?,
                                })
                            })
                            .collect::<Result<_>>()?,
                    })?;
                    active_add = None;
                    (
                        DagProposerAddBlockReport {
                            accepted: commit.accepted,
                            duplicate: false,
                            expired: false,
                            missing_references: Vec::new(),
                        },
                        commit.emit_verified,
                        commit.gossip && commit.accepted,
                    )
                };
                step = dag.report_proposer_add_block(session_id, add)?;
                (emit_verified, gossip)
            };
            if emit_verified {
                let effect_id = {
                    let _scope = mutation(&mut owner_generation)?;
                    self.next_effect_in_scope(generation)?
                };
                let _scope = callback(owner_generation)?;
                if let Ok(report) = observer.observe(&ConsensusObservationRequest {
                    effect_id,
                    kind: 2,
                    period: 0,
                    hash: signed.block_hash.0,
                    canonical_rlp: signed.block_rlp.clone(),
                }) {
                    self.validate_report_in_scope(effect_id, report.effect_id)?;
                }
            }
            if gossip {
                let effect_id = {
                    let _scope = mutation(&mut owner_generation)?;
                    self.next_effect_in_scope(generation)?
                };
                let request = GossipDagBlockRequest {
                    effect_id,
                    block_hash: signed.block_hash.0,
                    block_rlp: signed.block_rlp,
                };
                if let Some(egress_generation) = owner_generation {
                    self.send_persisted_proposal(
                        egress_generation,
                        vec![ProposalEgressPacket::Dag(request)],
                        pbft,
                        transport,
                        false,
                    )?;
                } else if let Ok(report) = transport.gossip_dag_block(&request) {
                    self.validate_report_in_scope(effect_id, report.effect_id)?;
                }
            }
            ensure!(
                matches!(step.action, DagProposerSessionAction::Complete),
                "CONSENSUS_DAG_PROPOSER_NOT_COMPLETE"
            );
            Ok(step.return_value)
        })();
        let Err(error) = result else {
            return result;
        };
        let vdf = active_vdf_job
            .map(|job_id| self.vdf_executor.cancel(job_id))
            .transpose();
        let cleanup = (|| {
            let _scope = self.composition.mutation()?;
            let add = active_add
                .map(|cursor_id| dag.abort_add_block(cursor_id))
                .transpose();
            let session = dag.abort_proposer_session(session_id);
            vdf?;
            add?;
            session?;
            Ok::<_, anyhow::Error>(())
        })();
        if let Err(cleanup_error) = cleanup {
            return Err(error.context(format!("CONSENSUS_DAG_PROPOSER_CLEANUP: {cleanup_error}")));
        }
        if error.is::<DagProposerStale>() {
            return Ok(false);
        }
        Err(error)
    }

    fn drive_application_finalization<E, S, T, O>(
        &self,
        generation: u64,
        request: &PbftApplicationFinalizationRequest,
        mut step: PbftApplicationFinalizationStep,
        services: ConsensusRuntimeServices<'_>,
        ports: FinalizationPorts<'_, S, T, E, O>,
    ) -> Result<bool>
    where
        E: ConsensusExecutionPort,
        S: ConsensusSigningPort,
        T: ConsensusTransportPort,
        O: ConsensusObserverPort,
    {
        self.composition
            .require_execution_plan("certified transition plan")?;
        let ConsensusRuntimeServices {
            pbft,
            dag,
            final_chain,
        } = services;
        let FinalizationPorts {
            signer,
            transport,
            evm,
            observer,
        } = ports;

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
                    let report = {
                        let __scope_arg_0_0 = &GossipPillarVoteRequest {
                            effect_id: id,
                            pillar_vote_rlp: gossip.pillar_vote_rlp.clone(),
                            rebroadcast: false,
                        };
                        self.composition
                            .callback()
                            .and_then(|_operation| transport.gossip_pillar_vote(__scope_arg_0_0))
                    }?;
                    self.validate_report(id, report.effect_id)?;
                    let succeeded = report.succeeded;
                    let failure = report.error_code.clone();
                    let next = report_pbft_application_finalization_pillar_gossip(
                        pbft,
                        dag,
                        final_chain,
                        request,
                        gossip,
                        crate::pbft_application_finalization::PbftApplicationPillarGossipReport {
                            succeeded: report.succeeded,
                            status: u8::from(!report.succeeded),
                            error_code: report.error_code,
                        },
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

    pub(crate) fn slashing_submitters(
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
        services: ConsensusRuntimeServices<'_>,
        ports: SlashingPorts<'_, P, S, T>,
        submitters: &[crate::SlashingSubmitterIdentity],
    ) -> Result<()>
    where
        P: ConsensusProcessPort,
        S: ConsensusSigningPort,
        T: ConsensusTransportPort,
    {
        self.composition
            .require_execution_plan("split slashing signing and submission")?;
        let ConsensusRuntimeServices {
            pbft,
            dag,
            final_chain,
        } = services;
        let SlashingPorts {
            process,
            signer,
            transport,
        } = ports;

        let leaves = RuntimeSyncedLeaves {
            runtime: self,
            generation,
            pbft,
            final_chain,
            process,
            signer,
            transport,
        };
        let inserted = {
            let __scope_arg_0_4 = &leaves;
            let __scope_arg_0_3 = submitters;
            let __scope_arg_0_2 = final_chain;
            let __scope_arg_0_1 = dag;
            let __scope_arg_0_0 = effect;
            let _operation = self.composition.mutation()?;
            pbft.submit_synced_slashing_transaction(
                __scope_arg_0_0,
                __scope_arg_0_1,
                __scope_arg_0_2,
                __scope_arg_0_3,
                __scope_arg_0_4,
                || Ok(()),
            )
        }?;
        let report = {
            let __scope_arg_0_1 = inserted;
            let __scope_arg_0_0 = effect.proof_hash;
            let _operation = self.composition.mutation()?;
            pbft.report_verified_vote_slashing_transaction_submission(
                __scope_arg_0_0,
                __scope_arg_0_1,
            )
        }?;
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
        production: Option<(u64, &PbftService)>,
    ) -> Result<Option<MaybeBroadcastVotesCommit>> {
        let mut production = production;
        let mut acknowledgements = Vec::with_capacity(batch.requests.len());
        for request in &batch.requests {
            let effect_id = self.next_effect_in_scope(generation)?;
            let packet = match request {
                ConsensusVoteTransportRequest::Vote {
                    canonical_vote_rlp,
                    proposed_block_rlp,
                    rebroadcast,
                    ..
                } => ProposalEgressPacket::Vote(GossipVoteRequest {
                    effect_id,
                    vote_rlp: canonical_vote_rlp.clone(),
                    proposed_block_rlp: proposed_block_rlp.clone().unwrap_or_default(),
                    rebroadcast: *rebroadcast,
                }),
                ConsensusVoteTransportRequest::VoteBundle {
                    canonical_votes_bundle_rlp,
                    rebroadcast,
                    ..
                } => ProposalEgressPacket::VoteBundle(GossipVoteBundleRequest {
                    effect_id,
                    votes_bundle_rlp: canonical_votes_bundle_rlp.clone(),
                    rebroadcast: *rebroadcast,
                }),
                ConsensusVoteTransportRequest::PillarVote {
                    canonical_pillar_vote_rlp,
                    rebroadcast,
                    ..
                } => ProposalEgressPacket::Pillar(GossipPillarVoteRequest {
                    effect_id,
                    pillar_vote_rlp: canonical_pillar_vote_rlp.clone(),
                    rebroadcast: *rebroadcast,
                }),
            };
            let (succeeded, error_code) = if let Some((owner_generation, pbft)) = production {
                match self.send_persisted_proposal(
                    owner_generation,
                    vec![packet],
                    pbft,
                    transport,
                    true,
                )? {
                    OwnedVoteTransport::Completed(next) => production = Some((next, pbft)),
                    OwnedVoteTransport::Stale | OwnedVoteTransport::Incomplete => return Ok(None),
                }
                (true, String::new())
            } else {
                let report = packet.send(transport)?;
                self.validate_report(effect_id, report.effect_id)?;
                (report.succeeded, report.error_code)
            };
            acknowledgements.push(VoteBroadcastAcknowledgement {
                request_id: request.request_id(),
                family: request.family(),
                succeeded,
                error_code,
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
        services: ConsensusRuntimeServices<'_>,
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
        self.composition.require_historical()?;
        self.run_inner(services, process, signer, transport, evm, observer)
    }

    fn recover_owned_pillar_anchor<E: ConsensusExecutionPort>(
        &self,
        generation: u64,
        pbft: &PbftService,
        final_chain: &FinalChain,
        evm: &E,
    ) -> Result<()> {
        use crate::pbft_application_finalization::{
            pillar_anchor_state_period, prepare_owner_pillar_anchor,
            process_pillar_after_finalization,
        };
        let (head, checkpoint) = {
            let _read = self.composition.callback()?;
            let head = final_chain.last_block_number_typed()?.as_u64();
            let (ficus, interval) = pbft.pillar_schedule();
            if !process_pillar_after_finalization(ficus, interval, head) {
                return Ok(());
            }
            let anchor = pbft.current_pillar_block_rlp()?;
            if !anchor.is_empty() && rustaxa_types::PillarBlock::decode_rlp(&anchor)?.period == head
            {
                return Ok(());
            }
            (
                head,
                pillar_anchor_state_period(head, final_chain.dpos_delegation_delay())?,
            )
        };
        let effect = self.next_effect(generation)?;
        let report =
            self.load_pillar_anchor_state(effect, checkpoint, head, Vec::new(), final_chain, evm)?;
        ensure!(
            report.succeeded,
            "PRODUCTION_STARTUP_PILLAR_CHECKPOINT:{}",
            report.error_code
        );
        let _operation = self.composition.mutation()?;
        ensure!(
            head == final_chain.last_block_number_typed()?.as_u64(),
            "PRODUCTION_STARTUP_PILLAR_HEAD_CHANGED"
        );
        let anchor = prepare_owner_pillar_anchor(
            pbft,
            final_chain,
            head,
            report.state_root,
            report.bridge_root,
            report.bridge_epoch,
        )?;
        pbft.apply_pillar_current_block_data_for_generation(anchor.data_rlp, anchor.generation)
    }

    fn dispatch_owned_pillar_votes<S: ConsensusSigningPort, T: ConsensusTransportPort>(
        &self,
        generation: u64,
        pbft: &PbftService,
        final_chain: &FinalChain,
        signer: &S,
        transport: &T,
        announced: &mut Option<ethereum_types::H256>,
    ) -> Result<()> {
        use crate::consensus_application_startup::StartupPillarVoteDraft;
        use rustaxa_types::{PillarBlock, PillarVote};
        let (binding, anchor, packets) = {
            let operation = self.composition.callback()?;
            let rlp = pbft.current_pillar_block_rlp()?;
            if rlp.is_empty() {
                return Ok(());
            }
            let block = PillarBlock::decode_rlp(&rlp)?;
            let anchor = block.hash();
            let period = block
                .period
                .checked_add(1)
                .context("PRODUCTION_PILLAR_VOTE_PERIOD_OVERFLOW")?;
            if *announced == Some(anchor) || pbft.manager_snapshot().period != period {
                return Ok(());
            }
            let eligibility_period = block.period.into();
            let total = final_chain.dpos_eligible_total_vote_count(eligibility_period)?;
            let retained = pbft.load_pillar_startup_bootstrap()?.own_vote_rlp;
            let retained = (!retained.is_empty())
                .then(|| PillarVote::decode_rlp(&retained))
                .transpose()?;
            let unsigned = PillarVote {
                period,
                block_hash: anchor,
                signature: [0; 65],
            };
            let digest = unsigned.hash(false);
            let mut packets = Vec::new();
            for identity in &self.signing_identities {
                let count =
                    final_chain.dpos_eligible_vote_count(eligibility_period, identity.address)?;
                if count == 0 {
                    continue;
                }
                let draft = StartupPillarVoteDraft {
                    period,
                    block_hash: anchor,
                    digest,
                    wallet_index: identity.wallet_index,
                    expected_voter: identity.address,
                    validator_vote_count: count,
                    total_eligible_vote_count: total,
                };
                let signature = if let Some(vote) = retained.as_ref().filter(|vote| {
                    vote.period == period
                        && vote.block_hash == anchor
                        && vote.recover_voter_address().map(Into::<[u8; 20]>::into)
                            == Some(identity.address)
                }) {
                    vote.signature.to_vec()
                } else {
                    let effect_id = self.next_effect_in_scope(generation)?;
                    let report = signer.sign_digest(&ConsensusSignRequest {
                        effect_id,
                        wallet_index: identity.wallet_index,
                        digest: digest.into(),
                    })?;
                    self.validate_report_in_scope(effect_id, report.effect_id)?;
                    Self::checked_signature(report)?
                };
                packets.push((draft, signature));
            }
            (operation.generation(), anchor, packets)
        };
        let (owner_generation, requests) = {
            let operation = self.composition.mutation()?;
            if !operation.follows(binding) {
                return Ok(());
            }
            let mut requests = Vec::with_capacity(packets.len());
            for (draft, signature) in packets {
                let pillar_vote_rlp = apply_startup_pillar_vote(&draft, &signature, pbft)?;
                requests.push(ProposalEgressPacket::Pillar(GossipPillarVoteRequest {
                    effect_id: self.next_effect_in_scope(generation)?,
                    pillar_vote_rlp,
                    rebroadcast: false,
                }));
            }
            (
                operation
                    .generation()
                    .context("PRODUCTION_PILLAR_EGRESS_SCOPE")?,
                requests,
            )
        };
        if matches!(
            self.send_persisted_proposal(owner_generation, requests, pbft, transport, true)?,
            OwnedVoteTransport::Completed(_)
        ) {
            *announced = Some(anchor);
        }
        Ok(())
    }

    fn run_inner<P, S, T, E, O>(
        &self,
        services: ConsensusRuntimeServices<'_>,
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
        let ConsensusRuntimeServices {
            pbft,
            dag: dag_transaction,
            final_chain,
        } = services;

        let generation = self.begin_run()?;
        struct RunningGuard<'a>(&'a AtomicBool, &'a ApplicationComposition);
        impl Drop for RunningGuard<'_> {
            fn drop(&mut self) {
                if let Ok(_operation) = self.1.mutation() {
                    self.0.store(false, Ordering::Release);
                }
            }
        }
        let _guard = RunningGuard(&self.running, &self.composition);
        if {
            let __scope_arg_0_0 = generation;
            let _operation = self.composition.callback()?;
            process.stop_requested(__scope_arg_0_0)
        } {
            return Ok(ConsensusRunExit {
                generation,
                reason: ConsensusRunReason::Stopped,
            });
        }
        if !{
            let _operation = self.composition.callback()?;
            pbft.is_ready()
        } {
            self.composition
                .require_execution_plan("startup pillar and replay preparation")?;
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
                {
                    let __scope_arg_0_0 = final_chain;
                    let _operation = self.composition.mutation()?;
                    pbft.finalization_ready(__scope_arg_0_0)
                }?,
                "CONSENSUS_STARTUP_FINALIZATION_NOT_READY"
            );
            hydrate_recently_finalized_transactions(&startup, dag_transaction)?;
            if let Some(persisted) = &startup.persisted_pillar_vote {
                loop {
                    if {
                        let __scope_arg_0_0 = generation;
                        let _operation = self.composition.callback()?;
                        process.stop_requested(__scope_arg_0_0)
                    } {
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
                            if self.wait_for(generation, self.polling_interval_ms, process, None)?
                                == RuntimeWaitOutcome::Stopped
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
                    let report = {
                        let __scope_arg_0_0 = &GossipPillarVoteRequest {
                            effect_id,
                            pillar_vote_rlp: vote_rlp,
                            rebroadcast: false,
                        };
                        self.composition
                            .callback()
                            .and_then(|_operation| transport.gossip_pillar_vote(__scope_arg_0_0))
                    }?;
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
        let production = matches!(self.composition, ApplicationComposition::Production(_));
        if production {
            self.recover_owned_pillar_anchor(generation, pbft, final_chain, evm)?;
        }
        let mut announced_pillar = None;
        let mut rejected_certificate = None;
        let mut tick_id = 0u64;
        let mut timing = ConsensusTimingOrigins::new({
            let _operation = self.composition.callback()?;
            process.now_millis()
        });
        'ticks: loop {
            if {
                let __scope_arg_0_0 = generation;
                let _operation = self.composition.callback()?;
                process.stop_requested(__scope_arg_0_0)
            } {
                return Ok(ConsensusRunExit {
                    generation,
                    reason: ConsensusRunReason::Stopped,
                });
            }
            if production {
                self.dispatch_owned_pillar_votes(
                    generation,
                    pbft,
                    final_chain,
                    signer,
                    transport,
                    &mut announced_pillar,
                )?;
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
            let (now_ms, transport_status, observed_generation) = {
                let operation = self.composition.callback()?;
                (
                    process.now_millis(),
                    transport.transport_status(),
                    operation.generation(),
                )
            };
            let tick_scope = self.composition.mutation()?;
            if !tick_scope.follows(observed_generation) {
                continue 'ticks;
            }
            let snapshot = pbft.manager_snapshot();
            timing.observe(snapshot.period, snapshot.round, now_ms);
            let network_sync_status = pbft.network_service().pbft_sync_status(now_ms)?;
            let addresses: Vec<_> = self
                .signing_identities
                .iter()
                .map(|id| id.address)
                .collect();
            let has_eligible_wallet = {
                let __scope_arg_0_1 = &addresses;
                let __scope_arg_0_0 = snapshot.period;
                let _operation = self.composition.mutation()?;
                final_chain.pbft_dpos_eligible_wallet_vote_counts(__scope_arg_0_0, __scope_arg_0_1)
            }?
            .is_some_and(|votes| votes.into_iter().any(|vote| vote.vote_count > 0));
            let session_identity = {
                let __scope_arg_0_0 = PbftManagerRuntimeTickFact {
                    tick_id,
                    state: snapshot.state,
                    period: snapshot.period,
                    round: snapshot.round,
                    step: snapshot.step,
                    network_available: transport_status.available,
                    network_pbft_syncing: network_sync_status.active,
                    has_eligible_wallet,
                    polling_interval_ms: self.polling_interval_ms,
                };
                let _operation = self.composition.mutation()?;
                pbft.begin_owned_runtime_session(__scope_arg_0_0)
            }?;
            let session = RuntimeSessionGuard {
                pbft,
                composition: &self.composition,
                identity: session_identity,
                discarded: Cell::new(false),
            };
            let mut session_generation = tick_scope.generation();
            drop(tick_scope);
            loop {
                let action_now_ms = {
                    let _callback = self.composition.callback()?;
                    process.now_millis()
                };
                let mut action_scope = Some(self.composition.mutation()?);
                if !action_scope
                    .as_ref()
                    .expect("action scope")
                    .follows(session_generation)
                {
                    session.discard()?;
                    continue 'ticks;
                }
                let action_generation = action_scope.as_ref().expect("action scope").generation();
                let step = {
                    let _operation = self.composition.mutation()?;
                    pbft.owned_runtime_session_next(&session.identity)
                }?;
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
                if !matches!(
                    action,
                    PbftManagerRuntimeAction::SleepIneligiblePollingInterval
                        | PbftManagerRuntimeAction::SleepUntilNextStep
                        | PbftManagerRuntimeAction::DelayCertifyPoll
                        | PbftManagerRuntimeAction::DelayFinishPoll
                ) && !(matches!(self.composition, ApplicationComposition::Production(_))
                    && matches!(
                        action,
                        PbftManagerRuntimeAction::ProcessSyncedPbftBlocks
                            | PbftManagerRuntimeAction::TryPushCertVotesBlock
                    ))
                {
                    drop(action_scope.take());
                }
                let mut waited_generation = None;
                let mut has_new_round = false;
                let mut new_round = 0;
                let mut go_finish_state = false;
                let mut loop_back_finish_state = false;
                let result = match action {
                    PbftManagerRuntimeAction::SleepIneligiblePollingInterval => {
                        drop(action_scope.take());
                        match self.wait_for(
                            generation,
                            step.sleep_ms,
                            process,
                            action_generation,
                        )? {
                            RuntimeWaitOutcome::Elapsed(epoch) => {
                                waited_generation = Some(epoch);
                            }
                            RuntimeWaitOutcome::Stopped => {
                                session.discard()?;
                                return Ok(ConsensusRunExit {
                                    generation,
                                    reason: ConsensusRunReason::Stopped,
                                });
                            }
                            RuntimeWaitOutcome::StaleWork => {
                                session.discard()?;
                                continue 'ticks;
                            }
                        }
                        PbftManagerRuntimeActionResultCode::SleepApplied
                    }
                    PbftManagerRuntimeAction::SleepUntilNextStep => {
                        let elapsed_ms = timing.round_elapsed_ms(action_now_ms);
                        let elapsed_ms = i64::try_from(elapsed_ms).map_err(|_| {
                            anyhow::anyhow!("CONSENSUS_RUNTIME_ROUND_ELAPSED_OVERFLOW")
                        })?;
                        let plan = {
                            let __scope_arg_0_0 = elapsed_ms;
                            let _operation = self.composition.mutation()?;
                            pbft.plan_runtime_sleep_until_next_step(__scope_arg_0_0)
                        };
                        ensure!(
                            plan.accepted,
                            "CONSENSUS_RUNTIME_SLEEP_PLAN_REJECTED: {}",
                            plan.error_code
                        );
                        drop(action_scope.take());
                        match if plan.should_sleep {
                            self.wait_for(generation, plan.sleep_ms, process, action_generation)?
                        } else {
                            RuntimeWaitOutcome::Elapsed(action_generation)
                        } {
                            RuntimeWaitOutcome::Elapsed(epoch) => {
                                waited_generation = Some(epoch);
                            }
                            RuntimeWaitOutcome::Stopped => {
                                session.discard()?;
                                return Ok(ConsensusRunExit {
                                    generation,
                                    reason: ConsensusRunReason::Stopped,
                                });
                            }
                            RuntimeWaitOutcome::StaleWork => {
                                session.discard()?;
                                continue 'ticks;
                            }
                        }
                        PbftManagerRuntimeActionResultCode::SleepApplied
                    }
                    PbftManagerRuntimeAction::DelayCertifyPoll
                    | PbftManagerRuntimeAction::DelayFinishPoll => {
                        let outcome = if action == PbftManagerRuntimeAction::DelayCertifyPoll {
                            {
                                let _operation = self.composition.mutation()?;
                                pbft.delay_certify_poll()
                            }?
                        } else {
                            {
                                let _operation = self.composition.mutation()?;
                                pbft.delay_finish_poll()
                            }?
                        };
                        ensure!(
                            outcome.status == PbftLifecycleActionStatus::Applied,
                            "CONSENSUS_RUNTIME_DELAY_REJECTED: {}",
                            outcome.error_code
                        );
                        drop(action_scope.take());
                        match self.wait_for(
                            generation,
                            self.polling_interval_ms,
                            process,
                            action_generation,
                        )? {
                            RuntimeWaitOutcome::Elapsed(epoch) => {
                                waited_generation = Some(epoch);
                            }
                            RuntimeWaitOutcome::Stopped => {
                                session.discard()?;
                                return Ok(ConsensusRunExit {
                                    generation,
                                    reason: ConsensusRunReason::Stopped,
                                });
                            }
                            RuntimeWaitOutcome::StaleWork => {
                                session.discard()?;
                                continue 'ticks;
                            }
                        }
                        PbftManagerRuntimeActionResultCode::SleepApplied
                    }
                    PbftManagerRuntimeAction::TryAdvanceRound => {
                        if let PbftRoundAdvanceActionOutcome::AdvanceTo(outcome) = {
                            let _operation = self.composition.mutation()?;
                            pbft.try_advance_round()
                        }? {
                            has_new_round = true;
                            new_round = outcome.new_round;
                        }
                        PbftManagerRuntimeActionResultCode::NoProgressContinue
                    }
                    PbftManagerRuntimeAction::MaybeBroadcastVotes => {
                        let (live, batch, owner_generation) = {
                            let operation = self.composition.callback()?;
                            let live = pbft.manager_snapshot();
                            let now = process.now_millis();
                            let batch = select_maybe_broadcast_votes(
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
                                        rebroadcast_reward_votes: live
                                            .rebroadcast_reward_votes_counter,
                                    },
                                },
                            )?;
                            (live, batch, operation.generation())
                        };
                        if let Some(batch) = batch {
                            let commit = self.dispatch_scheduled_gossip(
                                generation,
                                &batch,
                                transport,
                                owner_generation.map(|owner| (owner, pbft)),
                            )?;
                            if let Some(commit) = commit {
                                {
                                    let __scope_arg_0_5 = commit.counters.rebroadcast_reward_votes;
                                    let __scope_arg_0_4 = commit.counters.broadcast_reward_votes;
                                    let __scope_arg_0_3 = commit.counters.rebroadcast_votes;
                                    let __scope_arg_0_2 = commit.counters.broadcast_votes;
                                    let __scope_arg_0_1 = live.round;
                                    let __scope_arg_0_0 = live.period;
                                    let _operation = self.composition.mutation()?;
                                    pbft.apply_broadcast_counters_if_epoch(
                                        __scope_arg_0_0,
                                        __scope_arg_0_1,
                                        __scope_arg_0_2,
                                        __scope_arg_0_3,
                                        __scope_arg_0_4,
                                        __scope_arg_0_5,
                                    )
                                };
                            }
                        }
                        PbftManagerRuntimeActionResultCode::StateActionDone
                    }
                    PbftManagerRuntimeAction::TryPushCertVotesBlock => {
                        let live = { pbft.manager_snapshot() };
                        match {
                            let __scope_arg_0_2 = TwoTPlusOneVotedBlockType::CertVotedBlock;
                            let __scope_arg_0_1 = live.round;
                            let __scope_arg_0_0 = live.period;
                            let _operation = self.composition.mutation()?;
                            pbft.verified_votes_get_two_t_plus_one_voted_block_payloads(
                                __scope_arg_0_0,
                                __scope_arg_0_1,
                                __scope_arg_0_2,
                            )
                        }? {
                            None => PbftManagerRuntimeActionResultCode::NoProgressContinue,
                            Some(certified)
                                if rejected_certificate
                                    == Some((live.period, live.round, certified.block_hash)) =>
                            {
                                PbftManagerRuntimeActionResultCode::NoProgressContinue
                            }
                            Some(certified) => {
                                match {
                                    let __scope_arg_0_1 = certified.block_hash;
                                    let __scope_arg_0_0 = live.period;
                                    let _operation = self.composition.mutation()?;
                                    pbft.proposed_block(__scope_arg_0_0, __scope_arg_0_1)
                                } {
                                    None => PbftManagerRuntimeActionResultCode::NoProgressContinue,
                                    Some(proposed) if action_scope.is_some() => {
                                        let owner_generation = action_scope
                                            .as_ref()
                                            .and_then(|scope| scope.generation())
                                            .context("PRODUCTION_CERTIFICATE_SCOPE_REQUIRED")?;
                                        session.discard()?;
                                        drop(action_scope.take());
                                        let result = evm.publish_scheduled_certificate(
                                            ScheduledCertificate {
                                                period: live.period,
                                                round: live.round,
                                                block_rlp: &proposed.block_rlp,
                                                votes: &certified.votes,
                                                owner_generation,
                                            },
                                        );
                                        let published = match result {
                                            Ok(value) => value,
                                            Err(error)
                                                if error.is::<ScheduledCertificateRejected>() =>
                                            {
                                                rejected_certificate = Some((
                                                    live.period,
                                                    live.round,
                                                    certified.block_hash,
                                                ));
                                                continue 'ticks;
                                            }
                                            Err(error) => return Err(error),
                                        };
                                        if published {
                                            self.observe_production_publication(
                                                generation,
                                                live.period,
                                                pbft,
                                                final_chain,
                                                observer,
                                            );
                                        }
                                        if !published
                                            || self.composition.has_live_production_successor()
                                        {
                                            continue 'ticks;
                                        }
                                        return Ok(ConsensusRunExit {
                                            generation,
                                            reason: ConsensusRunReason::Completed,
                                        });
                                    }
                                    Some(proposed) if !proposed.is_valid => {
                                        PbftManagerRuntimeActionResultCode::NoProgressContinue
                                    }
                                    Some(proposed) => {
                                        self.composition.require_execution_plan(
                                            "certified preparation before writes",
                                        )?;
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
                                            ConsensusRuntimeServices {
                                                pbft,
                                                dag: dag_transaction,
                                                final_chain,
                                            },
                                            FinalizationPorts {
                                                signer,
                                                transport,
                                                evm,
                                                observer,
                                            },
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
                                {
                                    let __scope_arg_0_0 = step.target_round;
                                    let _operation = self.composition.mutation()?;
                                    pbft.reset_consensus(__scope_arg_0_0)
                                }?
                            }
                            PbftManagerRuntimeAction::TransitionToFilter => {
                                let _operation = self.composition.mutation()?;
                                pbft.transition_to_filter()
                            }?,
                            PbftManagerRuntimeAction::TransitionToCertify => {
                                let _operation = self.composition.mutation()?;
                                pbft.transition_to_certify()
                            }?,
                            PbftManagerRuntimeAction::TransitionToFinish => {
                                let _operation = self.composition.mutation()?;
                                pbft.transition_to_finish()
                            }?,
                            PbftManagerRuntimeAction::TransitionToFinishPolling => {
                                let _operation = self.composition.mutation()?;
                                pbft.transition_to_finish_polling()
                            }?,
                            PbftManagerRuntimeAction::LoopBackFinish => {
                                let _operation = self.composition.mutation()?;
                                pbft.loop_back_finish()
                            }?,
                            _ => unreachable!(),
                        };
                        ensure!(
                            outcome.status == PbftLifecycleActionStatus::Applied,
                            "CONSENSUS_RUNTIME_LIFECYCLE_REJECTED: {}",
                            outcome.error_code
                        );
                        if action == PbftManagerRuntimeAction::TransitionToFinishPolling {
                            timing.finish_poll_started_ms = action_now_ms;
                        }
                        PbftManagerRuntimeActionResultCode::TransitionApplied
                    }
                    PbftManagerRuntimeAction::RunValueProposal => {
                        let (live, proposed_voters) = {
                            let operation = self.composition.callback()?;
                            let live = pbft.manager_snapshot();
                            let mut voters = std::collections::HashSet::new();
                            if operation.generation().is_some() {
                                for record in pbft.verified_votes_own_vote_records()? {
                                    let vote =
                                        crate::pbft_vote_validation::inspect_canonical_pbft_vote(
                                            &record.vote_rlp,
                                        )?;
                                    ensure!(
                                        vote.signature_valid && vote.vote_hash == record.hash,
                                        "PRODUCTION_PROPOSAL_STORED_VOTE_IDENTITY_MISMATCH"
                                    );
                                    if vote.period == live.period
                                        && vote.round == live.round
                                        && vote.step == 1
                                    {
                                        ensure!(
                                            vote.vote_type == PbftVoteType::Propose,
                                            "PRODUCTION_PROPOSAL_STORED_VOTE_TYPE_MISMATCH"
                                        );
                                        voters.insert(vote.recovered_voter.0);
                                    }
                                }
                            }
                            (live, voters)
                        };
                        let mut eligible = Vec::new();
                        for identity in &self.signing_identities {
                            if proposed_voters.contains(&identity.address) {
                                continue;
                            }
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
                            let proposer = {
                                let __scope_arg_0_2 = proof;
                                let __scope_arg_0_1 = request;
                                let __scope_arg_0_0 = final_chain;
                                let _operation = self.composition.mutation()?;
                                pbft.complete_public_proposer_sortition(
                                    __scope_arg_0_0,
                                    __scope_arg_0_1,
                                    __scope_arg_0_2,
                                )
                            }?;
                            if proposer.accepted {
                                eligible.push(identity.wallet_index);
                            }
                        }
                        let timestamp = {
                            let _operation = self.composition.callback()?;
                            process.unix_time_seconds()
                        };
                        let (action, proposal_generation) = {
                            let operation = self.composition.mutation()?;
                            if operation.generation().is_some() && pbft.manager_snapshot() != live {
                                session.discard()?;
                                continue 'ticks;
                            }
                            (
                                compose_value_proposal(
                                    pbft,
                                    final_chain,
                                    dag_transaction,
                                    timestamp,
                                    eligible,
                                    operation.generation().is_none(),
                                )?,
                                operation.generation(),
                            )
                        };
                        let stale_proposal = Cell::new(false);
                        let prepare_proposal_vote =
                            |wallet_index: u64,
                             block_rlp: Vec<u8>|
                             -> Result<Option<PreparedProposalVote>> {
                                let checked = if let Some(owner_generation) = proposal_generation {
                                    match evm
                                        .check_proposal_transactions(&block_rlp, owner_generation)?
                                    {
                                        None => {
                                            stale_proposal.set(true);
                                            return Ok(None);
                                        }
                                        Some(ProposalExecutionCheck::Invalid) => return Ok(None),
                                        Some(ProposalExecutionCheck::Unavailable(reason)) => {
                                            bail!(
                                                "PRODUCTION_PROPOSAL_EXECUTION_UNAVAILABLE: {reason:?}"
                                            );
                                        }
                                        Some(ProposalExecutionCheck::Checked(proof)) => Some(proof),
                                    }
                                } else {
                                    None
                                };
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
                                let generated = if let Some(checked) = checked.as_ref() {
                                    let ApplicationComposition::Production(source) =
                                        &self.composition
                                    else {
                                        bail!("PRODUCTION_PROPOSAL_OWNER_MISSING");
                                    };
                                    let operation = self.composition.callback()?;
                                    if !source.check_proposal_vote(
                                        checked,
                                        &operation,
                                        &block_rlp,
                                        live.period,
                                        false,
                                    )? || pbft.manager_snapshot() != live
                                    {
                                        stale_proposal.set(true);
                                        return Ok(None);
                                    }
                                    let Some(generated) = self.sign_bound_pbft_vote(
                                        generation,
                                        final_chain,
                                        pbft,
                                        vrf,
                                        signer,
                                    )?
                                    else {
                                        return Ok(None);
                                    };
                                    generated
                                } else {
                                    let proof = self.prove_vrf(
                                        generation,
                                        wallet_index,
                                        vrf.message.clone(),
                                        signer,
                                    )?;
                                    let Some(signing) = {
                                        self.composition
                                            .require_execution_plan("prepare_state_vote_signing")?;
                                        let __scope_arg_0_2 = proof;
                                        let __scope_arg_0_1 = vrf;
                                        let __scope_arg_0_0 = final_chain;
                                        let _operation = self.composition.mutation()?;
                                        pbft.prepare_state_vote_signing(
                                            __scope_arg_0_0,
                                            __scope_arg_0_1,
                                            __scope_arg_0_2,
                                        )
                                    }?
                                    else {
                                        return Ok(None);
                                    };
                                    let signature = self.sign_digest(
                                        generation,
                                        wallet_index,
                                        signing.signing_hash.0,
                                        signer,
                                    )?;
                                    complete_pbft_vote_signing(signing, signature)?
                                };
                                let task = ConsensusStateVoteTask {
                                    period: live.period,
                                    round: live.round,
                                    step: 1,
                                    vote_type: PbftVoteType::Propose,
                                    block_hash: link.block_hash,
                                    proposed_block_rlp: block_rlp.clone(),
                                    commit: ConsensusStateVoteCommit::None,
                                };
                                Ok(Some(PreparedProposalVote {
                                    block_rlp,
                                    vote: generated,
                                    task,
                                    execution: checked,
                                }))
                            };
                        let gossip_rewards = |bundle: Vec<u8>| -> Result<()> {
                            if bundle.is_empty() {
                                return Ok(());
                            }
                            let effect_id = self.next_effect(generation)?;
                            let report = {
                                let __scope_arg_0_0 = &GossipVoteBundleRequest {
                                    effect_id,
                                    votes_bundle_rlp: bundle,
                                    rebroadcast: false,
                                };
                                self.composition.callback().and_then(|_operation| {
                                    transport.gossip_vote_bundle(__scope_arg_0_0)
                                })
                            }?;
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
                                    let identity = self
                                        .signing_identities
                                        .get(wallet_index as usize)
                                        .filter(|identity| identity.wallet_index == wallet_index)
                                        .context("CONSENSUS_PROPOSAL_SIGNER_MISSING")?;
                                    let signature = if let Some(expected) = proposal_generation {
                                        let operation = self.composition.callback()?;
                                        if operation.generation() != Some(expected)
                                            || pbft.manager_snapshot() != live
                                        {
                                            drop(operation);
                                            session.discard()?;
                                            continue 'ticks;
                                        }
                                        let effect_id = self.next_effect_in_scope(generation)?;
                                        let report = signer.sign_digest(&ConsensusSignRequest {
                                            effect_id,
                                            wallet_index,
                                            digest: unsigned.signing_hash,
                                        })?;
                                        self.validate_report_in_scope(effect_id, report.effect_id)?;
                                        Self::checked_signature(report)?
                                    } else {
                                        self.sign_digest(
                                            generation,
                                            wallet_index,
                                            unsigned.signing_hash,
                                            signer,
                                        )?
                                    };
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
                                        metadata.author.0 == identity.address,
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
                        if stale_proposal.get() {
                            session.discard()?;
                            continue 'ticks;
                        }
                        if !candidates.is_empty() {
                            let owner_write = if let ApplicationComposition::Production(source) =
                                &self.composition
                            {
                                let operation = self.composition.mutation()?;
                                let mut current = pbft.manager_snapshot() == live;
                                for candidate in &candidates {
                                    let proof = candidate
                                        .execution
                                        .as_ref()
                                        .context("PRODUCTION_PROPOSAL_EXECUTION_PROOF_MISSING")?;
                                    current &= source.check_proposal_vote(
                                        proof,
                                        &operation,
                                        &candidate.block_rlp,
                                        candidate.task.period,
                                        true,
                                    )?;
                                }
                                if !current {
                                    drop(operation);
                                    session.discard()?;
                                    continue 'ticks;
                                }
                                Some(operation)
                            } else {
                                None
                            };
                            let production = owner_write.is_some();
                            let selected_candidates = if publish_selected_block {
                                let policy = {
                                    let __scope_arg_0_1 = ethereum_types::H256::zero();
                                    let __scope_arg_0_0 = live.period;
                                    let _operation = self.composition.mutation()?;
                                    pbft.value_proposal_admission_request(
                                        __scope_arg_0_0,
                                        __scope_arg_0_1,
                                    )
                                };
                                let selection = {
                                    let __scope_arg_0_2 = PbftLocalProposalSelectionRequest {
                                        candidates: candidates
                                            .iter()
                                            .map(|candidate| PbftLocalProposalCandidate {
                                                block_rlp: candidate.block_rlp.clone(),
                                                vote_rlp: candidate.vote.vote_rlp.clone(),
                                            })
                                            .collect(),
                                        period: live.period,
                                        round: live.round,
                                        pbft_gas_limit: policy.pbft_gas_limit,
                                        extra_data_required: policy.extra_data_required,
                                        pillar_block_required: policy.pillar_block_required,
                                    };
                                    let __scope_arg_0_1 = dag_transaction;
                                    let __scope_arg_0_0 = final_chain;
                                    let _operation = self.composition.mutation()?;
                                    pbft.select_local_proposal_candidate(
                                        __scope_arg_0_0,
                                        __scope_arg_0_1,
                                        __scope_arg_0_2,
                                    )
                                }?;
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
                            let mut production_egress = Vec::new();
                            if !selected_candidates.is_empty() {
                                if !production {
                                    gossip_rewards(std::mem::take(&mut reward_votes_bundle_rlp))?;
                                }
                                for PreparedProposalVote {
                                    block_rlp,
                                    vote: generated,
                                    task,
                                    ..
                                } in selected_candidates
                                {
                                    if publish_selected_block {
                                        {
                                            let __scope_arg_0_0 = block_rlp.clone();
                                            let _operation = self.composition.mutation()?;
                                            pbft.publish_proposed_block_effect(__scope_arg_0_0)
                                        }?;
                                    }
                                    let resolved_submitters = RefCell::new(None);
                                    let admission = {
                                        let __scope_arg_0_2 = &generated;
                                        let __scope_arg_0_1 = &task;
                                        let __scope_arg_0_0 = final_chain;
                                        let _operation = self.composition.mutation()?;
                                        pbft.admit_state_vote_with_slashing_resolver(
                                            __scope_arg_0_0,
                                            __scope_arg_0_1,
                                            __scope_arg_0_2,
                                            || {
                                                let submitters =
                                                    self.slashing_submitters(final_chain)?;
                                                resolved_submitters
                                                    .replace(Some(submitters.clone()));
                                                Ok(submitters)
                                            },
                                        )
                                    }?;
                                    ensure!(
                                        admission.transaction.transition_published,
                                        "CONSENSUS_PROPOSAL_VOTE_ADMISSION_NOT_PUBLISHED: {}",
                                        admission.transaction.persistence_error_code
                                    );
                                    if let Some(effect) =
                                        admission.slashing_transaction_effect.as_ref()
                                    {
                                        self.composition.require_execution_plan(
                                            "split slashing signing and submission",
                                        )?;
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
                                            ConsensusRuntimeServices {
                                                pbft,
                                                dag: dag_transaction,
                                                final_chain,
                                            },
                                            SlashingPorts {
                                                process,
                                                signer,
                                                transport,
                                            },
                                            submitters,
                                        )?;
                                    }
                                    if production && admission.validation.accepted {
                                        if !reward_votes_bundle_rlp.is_empty() {
                                            production_egress.push(
                                                ProposalEgressPacket::VoteBundle(
                                                    GossipVoteBundleRequest {
                                                        effect_id: self
                                                            .next_effect_in_scope(generation)?,
                                                        votes_bundle_rlp: std::mem::take(
                                                            &mut reward_votes_bundle_rlp,
                                                        ),
                                                        rebroadcast: false,
                                                    },
                                                ),
                                            );
                                        }
                                        production_egress.push(ProposalEgressPacket::Vote(
                                            GossipVoteRequest {
                                                effect_id: self.next_effect_in_scope(generation)?,
                                                vote_rlp: generated.vote_rlp,
                                                proposed_block_rlp: block_rlp,
                                                rebroadcast: false,
                                            },
                                        ));
                                    } else if admission.validation.accepted {
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
                            if let Some(write) = owner_write {
                                let owner_generation = write
                                    .generation()
                                    .context("PRODUCTION_PROPOSAL_EGRESS_GENERATION_MISSING")?;
                                drop(write);
                                self.send_persisted_proposal(
                                    owner_generation,
                                    production_egress,
                                    pbft,
                                    transport,
                                    false,
                                )?;
                            }
                        }
                        PbftManagerRuntimeActionResultCode::StateActionDone
                    }
                    PbftManagerRuntimeAction::RunFilter
                    | PbftManagerRuntimeAction::RunCertify
                    | PbftManagerRuntimeAction::RunFirstFinish
                    | PbftManagerRuntimeAction::RunSecondFinish
                        if action_generation.is_some() =>
                    {
                        let state = match action {
                            PbftManagerRuntimeAction::RunFilter => {
                                PbftManagerRuntimeStateCode::Filter
                            }
                            PbftManagerRuntimeAction::RunCertify => {
                                PbftManagerRuntimeStateCode::Certify
                            }
                            PbftManagerRuntimeAction::RunFirstFinish => {
                                PbftManagerRuntimeStateCode::Finish
                            }
                            PbftManagerRuntimeAction::RunSecondFinish => {
                                PbftManagerRuntimeStateCode::FinishPolling
                            }
                            _ => unreachable!(),
                        };
                        let Some((go_finish, loop_back)) = self.run_bound_state_vote(
                            generation,
                            action_generation.expect("production action generation"),
                            state,
                            timing.state_action_timing(action_now_ms),
                            ConsensusRuntimeServices {
                                pbft,
                                dag: dag_transaction,
                                final_chain,
                            },
                            signer,
                            transport,
                            evm,
                        )?
                        else {
                            session.discard()?;
                            continue 'ticks;
                        };
                        go_finish_state = go_finish;
                        loop_back_finish_state = loop_back;
                        PbftManagerRuntimeActionResultCode::StateActionDone
                    }
                    PbftManagerRuntimeAction::RunFilter
                    | PbftManagerRuntimeAction::RunCertify
                    | PbftManagerRuntimeAction::RunFirstFinish
                    | PbftManagerRuntimeAction::RunSecondFinish => {
                        self.composition
                            .require_execution_plan("state vote composition")?;
                        let batch = compose_consensus_state_action(
                            pbft,
                            final_chain,
                            dag_transaction,
                            timing.state_action_timing({
                                let _operation = self.composition.callback()?;
                                process.now_millis()
                            }),
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
                                let Some(signing) = {
                                    self.composition
                                        .require_execution_plan("prepare_state_vote_signing")?;
                                    let __scope_arg_0_2 = proof;
                                    let __scope_arg_0_1 = vrf;
                                    let __scope_arg_0_0 = final_chain;
                                    let _operation = self.composition.mutation()?;
                                    pbft.prepare_state_vote_signing(
                                        __scope_arg_0_0,
                                        __scope_arg_0_1,
                                        __scope_arg_0_2,
                                    )
                                }?
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
                                let admission = {
                                    self.composition.require_execution_plan(
                                        "admit_state_vote_with_slashing_resolver",
                                    )?;
                                    let __scope_arg_0_2 = &generated;
                                    let __scope_arg_0_1 = &task;
                                    let __scope_arg_0_0 = final_chain;
                                    let _operation = self.composition.mutation()?;
                                    pbft.admit_state_vote_with_slashing_resolver(
                                        __scope_arg_0_0,
                                        __scope_arg_0_1,
                                        __scope_arg_0_2,
                                        || {
                                            let submitters =
                                                self.slashing_submitters(final_chain)?;
                                            resolved_submitters.replace(Some(submitters.clone()));
                                            Ok(submitters)
                                        },
                                    )
                                }?;
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
                                        ConsensusRuntimeServices {
                                            pbft,
                                            dag: dag_transaction,
                                            final_chain,
                                        },
                                        SlashingPorts {
                                            process,
                                            signer,
                                            transport,
                                        },
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
                            {
                                self.composition
                                    .require_execution_plan("commit_state_vote_task")?;
                                let __scope_arg_0_0 = &task;
                                let _operation = self.composition.mutation()?;
                                pbft.commit_state_vote_task(__scope_arg_0_0)
                            }?;
                        }
                        PbftManagerRuntimeActionResultCode::StateActionDone
                    }
                    PbftManagerRuntimeAction::ProcessSyncedPbftBlocks => {
                        if matches!(self.composition, ApplicationComposition::Production(_)) {
                            let queue = pbft.certified_sync_front();
                            if let Some(queue) = queue {
                                session.discard()?;
                                drop(action_scope.take());
                                let leaves = RuntimeSyncedLeaves {
                                    runtime: self,
                                    generation,
                                    pbft,
                                    final_chain,
                                    process,
                                    signer,
                                    transport,
                                };
                                let mutation = || {
                                    let operation = self.composition.mutation()?;
                                    if !pbft.sync_front_matches(&queue) {
                                        return Err(crate::ScheduledCertificateStale.into());
                                    }
                                    Ok(operation)
                                };
                                let accepted = match pbft.admit_synced_period(
                                    queue.clone(),
                                    dag_transaction,
                                    final_chain,
                                    &mut || self.slashing_submitters(final_chain),
                                    &leaves,
                                    mutation,
                                ) {
                                    Ok(accepted) => accepted,
                                    Err(error)
                                        if error.is::<crate::ScheduledCertificateStale>() =>
                                    {
                                        continue 'ticks;
                                    }
                                    Err(error) => return Err(error),
                                };
                                let Some(accepted) = accepted else {
                                    let _operation = self.composition.mutation()?;
                                    if pbft.sync_front_matches(&queue) {
                                        pbft.acknowledge_synced_period(&queue)?;
                                    }
                                    continue 'ticks;
                                };
                                let first = accepted
                                    .current_cert_vote_rlps
                                    .first()
                                    .context("PRODUCTION_SYNC_CERTIFICATE_MISSING")?;
                                let first = inspect_canonical_pbft_vote(first)?;
                                let votes = accepted
                                    .current_cert_vote_rlps
                                    .into_iter()
                                    .map(|vote_rlp| {
                                        let vote = inspect_canonical_pbft_vote(&vote_rlp)?;
                                        Ok(crate::pbft_vote_payload::PbftVotePayloadRecord {
                                            hash: vote.vote_hash,
                                            vote_rlp,
                                        })
                                    })
                                    .collect::<Result<Vec<_>>>()?;
                                let owner_generation = {
                                    let scope = self.composition.callback()?;
                                    if !pbft.sync_front_matches(&queue) {
                                        continue 'ticks;
                                    }
                                    scope
                                        .generation()
                                        .context("PRODUCTION_SYNC_GENERATION_MISSING")?
                                };
                                let selected = ScheduledSyncedCertificate {
                                    queue,
                                    round: first.round,
                                    step: first.step,
                                    votes,
                                    owner_generation,
                                };
                                match evm.publish_synced_certificate(&selected) {
                                    Err(error)
                                        if error.is::<SyncedCertificateRejected>()
                                            || error
                                                .is::<crate::ProductionCertificateProofError>() =>
                                    {
                                        let operation = self.composition.mutation()?;
                                        if pbft.sync_front_matches(&selected.queue) {
                                            pbft.clear_period_data_queue();
                                            drop(operation);
                                            leaves.report_malicious_peer(
                                                selected.queue.source_peer_id,
                                            )?;
                                        }
                                    }
                                    Ok(true) => self.observe_production_publication(
                                        generation,
                                        selected.queue.entry_period,
                                        pbft,
                                        final_chain,
                                        observer,
                                    ),
                                    Ok(false) => {}
                                    Err(error) => return Err(error),
                                }
                                continue 'ticks;
                            }
                            PbftManagerRuntimeActionResultCode::StateActionDone
                        } else {
                            let leaves = RuntimeSyncedLeaves {
                                runtime: self,
                                generation,
                                pbft,
                                final_chain,
                                process,
                                signer,
                                transport,
                            };
                            let outcome = match {
                                self.composition
                                    .require_execution_plan("process_synced_pbft_blocks")?;
                                let __scope_arg_0_3 = &leaves;
                                let __scope_arg_0_1 = final_chain;
                                let __scope_arg_0_0 = dag_transaction;
                                let _operation = self.composition.mutation()?;
                                pbft.process_synced_pbft_blocks(
                                    __scope_arg_0_0,
                                    __scope_arg_0_1,
                                    || self.slashing_submitters(final_chain),
                                    __scope_arg_0_3,
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
                                            ConsensusRuntimeServices {
                                                pbft,
                                                dag: dag_transaction,
                                                final_chain,
                                            },
                                            FinalizationPorts {
                                                signer,
                                                transport,
                                                evm,
                                                observer,
                                            },
                                        )
                                    },
                                )
                            } {
                                Ok(outcome) => outcome,
                                Err(error) => return Err(error),
                            };
                            if outcome.finalized_entries > 0 {
                                PbftManagerRuntimeActionResultCode::ProgressRestartLoop
                            } else {
                                PbftManagerRuntimeActionResultCode::StateActionDone
                            }
                        }
                    }
                    PbftManagerRuntimeAction::Unknown => {
                        bail!("CONSENSUS_RUNTIME_UNKNOWN_ACTION: {action:?}")
                    }
                };
                let reported = {
                    let __scope_arg_0_0 = PbftManagerRuntimeActionReport {
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
                    };
                    let _operation = self.composition.mutation()?;
                    if waited_generation.is_some_and(|previous| !_operation.follows(previous)) {
                        session.discard()?;
                        continue 'ticks;
                    }
                    session_generation = _operation.generation();
                    pbft.report_owned_runtime_session(&session.identity, __scope_arg_0_0)
                }?;
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
                error_code: if self.stale {
                    "INJECTED_STALE_OBSERVER_REPORT".to_owned()
                } else {
                    String::new()
                },
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
                signature: if self.succeed {
                    vec![9; 65]
                } else {
                    Vec::new()
                },
                error_code: if self.succeed {
                    String::new()
                } else {
                    "SIGN_REJECTED".into()
                },
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
                error_code: if self.succeed {
                    String::new()
                } else {
                    "TRANSPORT_REJECTED".into()
                },
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
                error_code: if call == 0 {
                    "TRANSPORT_REJECTED".to_owned()
                } else {
                    String::new()
                },
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
                    },
                    None,
                )
                .unwrap(),
            RuntimeWaitOutcome::Stopped
        );
        assert!(
            runtime
                .wait_for(
                    generation,
                    5,
                    &FakeProcess {
                        outcome: ConsensusWaitOutcome::Elapsed,
                        stale: true
                    },
                    None,
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
                .dispatch_scheduled_gossip(generation, &batch, &transport, None)
                .unwrap(),
            None
        );
        assert_eq!(
            runtime
                .dispatch_scheduled_gossip(generation, &batch, &transport, None)
                .unwrap()
                .unwrap()
                .counters,
            expected
        );
        assert_eq!(transport.calls.load(Ordering::Acquire), 2);
    }
}
