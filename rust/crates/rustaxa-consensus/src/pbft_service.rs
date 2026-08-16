//! Native PBFT application-service composition and lifecycle ownership.
//!
//! [`PbftService`] restores and publishes the complete PBFT sibling graph from
//! one shared storage handle. Each sibling retains its own synchronization
//! domain; this root owns composition and bootstrap readiness only and never
//! adds a root-wide lock.

use crate::FinalChain;
use crate::dag::{DagBlockPeriodStorageLookup, dag_block_period_from_storage};
use crate::dag_transaction_service::DagTransactionService;
use crate::network_api::{
    ConsensusNetworkService, NETWORK_INGRESS_STATUS_ACCEPTED,
    NETWORK_INGRESS_STATUS_PBFT_SYNC_COMPLETE, NETWORK_INGRESS_STATUS_PBFT_SYNC_DUPLICATE_BLOCK,
    NETWORK_INGRESS_STATUS_PBFT_SYNC_MALICIOUS,
};
use crate::pbft_chain::{
    PbftBlockValidation, PbftChainHead, PbftChainService, pbft_block_exists_in_storage,
};
use crate::pbft_finalize::{
    PbftDynamicLambdaFact, PbftDynamicLambdaPlan, PbftFinalizationIntentFact,
    PbftFinalizationPeriodLambdaLookup, PbftFinalizationPlan, PbftFinalizationRuntimeAction,
    PbftFinalizationStatus, PbftFinalizedPeriodApplyResult,
    load_pbft_finalization_last_period_lambda, plan_pbft_dynamic_lambda,
    plan_pbft_finalization_intent,
};
use crate::pbft_manager::{
    PbftCandidateDagPreparationStatus, PbftFinalizationExecutorBoundary,
    PbftFinalizationExecutorStartRequest, PbftFinalizationOwnedActionDrain,
    PbftManagerAdvancePeriodPlan, PbftManagerGuard, PbftManagerLifecycleTransitionRequest,
    PbftManagerProposalAction, PbftManagerProposalDagBlockFact, PbftManagerProposalDagOrderReport,
    PbftManagerProposalInitialFact, PbftManagerProposalSessionStep, PbftManagerRuntimeActionReport,
    PbftManagerRuntimeSessionStep, PbftManagerRuntimeSnapshot, PbftManagerRuntimeTickFact,
    PbftManagerService, PbftManagerSleepPlan, PbftManagerStartupReplayPeriod,
    PbftManagerStateActionEffectReport, PbftManagerStateActionFact,
    PbftManagerStateActionSessionStep, PbftManagerStorageStartupFact, PbftManagerTransitionStatus,
    PbftManagerTransitionStorageStatus, abort_pbft_manager_runtime_session,
    apply_executed_block_reset_storage, apply_next_voted_status_storage,
    apply_pbft_manager_cursor_field_storage, apply_pbft_manager_transition_storage,
    base_owned_finalization_live_report, create_pbft_manager_proposal_session,
    create_pbft_manager_runtime_from_storage, create_pbft_manager_runtime_session,
    create_pbft_manager_state_action_effect_session, load_pbft_manager_startup_replay_period,
    next_pbft_manager_proposal_session, next_pbft_manager_runtime_action,
    next_pbft_manager_state_action_effect_session, plan_pbft_manager_runtime_sleep_until_next_step,
    report_pbft_manager_proposal_dag_order, report_pbft_manager_runtime_action,
    report_pbft_manager_state_action_effect_session, save_cert_voted_block_in_round_storage,
    stale_pbft_manager_proposal_session_step,
};
#[cfg(test)]
use crate::pbft_period_cleanup::{PbftPeriodStateCleanupResult, cleanup_period_state_with_commit};
use crate::pbft_period_cleanup::{
    PbftPeriodStateCleanupStatus, cleanup_period_state_with_commit_and_publish,
};
use crate::pbft_readiness::PbftServiceReadiness;
use crate::pbft_sync::{
    PbftSyncCertVoteBundleFact, PbftSyncCertVoteBundleStatus, PbftSyncCertVoteBundleValidation,
    PbftSyncCertVoteFact, PbftSyncQueueDrainReport, PbftSyncQueueDrainReportResult,
    PbftSyncQueueDrainStep, create_pbft_sync_queue_drain_session, next_pbft_sync_queue_drain_step,
    report_pbft_sync_queue_drain_step, validate_pbft_sync_cert_vote_bundle,
};
use crate::pbft_thresholds::{
    PbftTwoTPlusOneThresholdFact, PbftTwoTPlusOneThresholdPlan, PbftTwoTPlusOneThresholdStatus,
};
use crate::pbft_vote_event::PbftVoteEventFactFlags;
use crate::pbft_vote_generation::{
    PbftFinalChainDposAddressVoteFact, PbftFinalChainDposTotalVoteCountFacts,
    PbftFinalChainDposTotalVoteCountRequest, PbftFinalChainDposWalletAggregateVoteCountFacts,
    PbftFinalChainDposWalletAggregateVoteCountRequest,
    PbftFinalChainDposWalletEligibilityBatchFacts, PbftFinalChainDposWalletEligibilityBatchRequest,
    PbftFinalChainDposWalletEligibilityFacts, PbftFinalChainDposWalletEligibilityRequest,
    PbftFinalChainFact, PbftGeneratedVote, PbftVoteGenerationInput, PbftVoteWeightFacts,
    generate_pbft_vote_with_weight,
};
use crate::pbft_vote_payload::{
    build_slashing_pbft_vote_payload, build_weighted_pbft_vote_payload,
};
use crate::pbft_vote_progress::PbftVoteProgressContext;
use crate::pbft_vote_runtime::{
    PbftNextVotesBundleEgressPayloads, PbftNextVotesBundleEgressPlan,
    PbftOptimizedVoteBundleBuildRequest, PbftOptimizedVoteBundleBuildResult,
    PbftRewardVotePayloadSelection, PbftVerifiedVoteProgressPersistenceWrite,
    PbftVerifiedVotesService, PbftVoteAdmissionPersistenceStatus,
    PbftVoteAdmissionTransactionResult, PbftVoteRuntimeReplayOutcome, RewardVoteCursorSnapshot,
    RewardVotePayloadSnapshot, RewardVoteResetApplyRequest, VerifiedStepVotePayloadEntry,
    VerifiedVotesStateSnapshot, VerifiedVotesTwoTPlusOneVotePayloads,
    VerifiedVotesTwoTPlusOneVotedBlock,
};
use crate::pbft_vote_storage::{
    PbftVotePersistenceResult, PbftVoteStorageRecord, persist_local_vote_admission,
    persist_pbft_vote_progress,
};
use crate::pbft_vote_validation::{
    PbftCanonicalVoteInspectionStatus, PbftCanonicalVoteValidation, PbftProposerSortitionRequest,
    PbftProposerSortitionResult, PbftVoteAdmissionValidationRequest,
    PbftVoteValidationExternalFacts,
    generate_and_validate_proposer_sortition_with_prepared_request, inspect_canonical_pbft_vote,
    prepare_and_validate_pbft_proposer_sortition_request, validate_canonical_pbft_vote,
};
use crate::period_data_queue::{
    DecodedPbftSyncPacketPrecheck, EncodedPeriodDataQueuePushRequest, PeriodDataQueuePopPlan,
    PeriodDataQueuePushOutcome, PeriodDataQueuePushRequest, PeriodDataQueueSnapshot,
    PeriodDataQueueTransactionIdentity, decode_pbft_sync_packet_precheck,
};
use crate::pillar_chain::{
    PillarBlockLinkagePlan, PillarCurrentAnchorDecisionRequest, PillarValidatorVoteCount,
    load_own_pillar_block_vote_storage,
};
use crate::pillar_chain_service::{
    PillarBlockCreationRequest, PillarBlockCreationWithVoteCountsPlan, PillarBlockLinkageRequest,
    PillarChainService, PillarChainStartupBootstrap, PillarCurrentAnchorDecisionResult,
};
use crate::pillar_vote_service::{
    PillarBlockFinalizationAcknowledgeRequest, PillarBlockFinalizationAcknowledgeResult,
    PillarBlockFinalizationPrepareResult, PillarBlockFinalizationRequest,
    PillarConsensusThresholdLookup, PillarVoteBundleWithFinalChainPlan, PillarVoteRelevancePlan,
    PillarVoteRlpPayload, PillarVoteRuntimeRelevanceContext, PillarVoteSingleAdmissionContext,
    PillarVoteSingleAdmissionValidationPlan, PillarVoteSingleAdmissionWithFinalChainPlan,
    PillarVotesPayloadLookup,
};
use crate::proposed_blocks::{ProposedBlockEntry, ProposedBlocksService};
use crate::slashing::{
    DoubleVotingProofInput, DoubleVotingProofSubmissionPlan, SlashingProofService,
    SlashingSubmitterFact, SlashingSubmitterIdentity, SlashingTransactionEffect,
};
use crate::verified_votes::{DetermineNewRoundOutcome, PbftVoteType, TwoTPlusOneVotedBlockType};
use anyhow::{Context, Result, anyhow, ensure};
use ethereum_types::{H256, U256};
use rand::Rng;
use rlp::{Rlp, RlpStream};
use rustaxa_storage::{Storage, StorageWriteBatch};
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::pbft::{PbftBlockLink, PbftBlockMetadata};
use std::sync::{Arc, Mutex};
use tiny_keccak::{Hasher, Keccak};

const SLASHING_PROOF_CACHE_MAX_SIZE: usize = 1000;
const SLASHING_PROOF_CACHE_DELETE_STEP: usize = 100;

fn pbft_candidate_dag_order_hash(
    payload: &crate::dag_service::DagRuntimeNonFinalizedSyncPayload,
) -> H256 {
    let mut stream = RlpStream::new_list(1);
    stream.begin_list(payload.storage.blocks.len());
    for block in &payload.storage.blocks {
        let bytes: &[u8] = block.hash.as_bytes();
        stream.append(&bytes);
    }
    let mut out = [0; 32];
    let mut hasher = Keccak::v256();
    hasher.update(&stream.out());
    hasher.finalize(&mut out);
    H256(out)
}

fn decode_pbft_proposed_block_extra_data(block: &Rlp<'_>) -> Result<(bool, Option<H256>)> {
    if block.item_count()? != 9 {
        return Ok((false, None));
    }

    let bytes = block.at(7)?.data()?;
    ensure!(
        bytes.len() <= 1024,
        "PBFT_PROPOSED_BLOCK_EXTRA_DATA_TOO_LARGE"
    );
    let extra = Rlp::new(bytes);
    if extra.item_count().ok() != Some(6)
        || extra.val_at::<u16>(0).is_err()
        || extra.val_at::<u16>(1).is_err()
        || extra.val_at::<u16>(2).is_err()
        || extra.val_at::<u16>(3).is_err()
        || extra.val_at::<Vec<u8>>(4).is_err()
    {
        return Ok((false, None));
    }

    let pillar_hash = match extra.at(5).and_then(|value| value.data()) {
        Ok([]) => None,
        Ok(data) if data.len() == 32 => Some(H256::from_slice(data)),
        Ok(_) | Err(_) => return Ok((false, None)),
    };
    Ok((true, pillar_hash))
}

pub(crate) fn proposed_block_validation_candidate(
    entry: &ProposedBlockEntry,
    request: PbftProposedBlockAdmissionRequest,
) -> Result<PbftBlockValidationCandidate> {
    let block = Rlp::new(&entry.block_rlp);
    let item_count = block.item_count()?;
    ensure!(
        matches!(item_count, 8 | 9),
        "PBFT_PROPOSED_BLOCK_INVALID_FIELD_COUNT"
    );
    let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&entry.block_rlp))?;
    let metadata = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(&entry.block_rlp))?;
    ensure!(
        entry.period == request.period
            && link.period == request.period
            && metadata.period == request.period,
        "PBFT_PROPOSED_BLOCK_PERIOD_MISMATCH"
    );
    ensure!(
        entry.block_hash == request.block_hash && link.block_hash == request.block_hash,
        "PBFT_PROPOSED_BLOCK_HASH_MISMATCH"
    );
    ensure!(
        entry.pivot_hash == link.pivot_dag_block_hash,
        "PBFT_PROPOSED_BLOCK_PIVOT_MISMATCH"
    );

    let reward_votes = block.at(6)?;
    ensure!(
        reward_votes.is_list(),
        "PBFT_PROPOSED_BLOCK_REWARD_VOTES_NOT_LIST"
    );
    let mut reward_vote_hashes = Vec::with_capacity(reward_votes.item_count()?);
    for index in 0..reward_votes.item_count()? {
        let hash: H256 = reward_votes.val_at(index)?;
        ensure!(
            !reward_vote_hashes.contains(&hash),
            "PBFT_PROPOSED_BLOCK_DUPLICATE_REWARD_VOTE"
        );
        reward_vote_hashes.push(hash);
    }
    let (extra_data_present, pillar_block_hash) = decode_pbft_proposed_block_extra_data(&block)?;

    Ok(PbftBlockValidationCandidate {
        fact: crate::pbft_manager::PbftManagerBlockValidationFact {
            block_hash: link.block_hash,
            period: link.period,
            pivot_hash: link.pivot_dag_block_hash,
            pivot_is_null: link.pivot_dag_block_hash == H256::zero(),
            dag_order_required: true,
            extra_data_required: request.extra_data_required,
            extra_data_present,
            extra_data_pillar_hash_present: pillar_block_hash.is_some(),
            pillar_block_required: request.pillar_block_required,
            pbft_chain_status:
                crate::pbft_manager::PbftManagerBlockValidationFactStatus::NotChecked,
            final_chain_hash_status:
                crate::pbft_manager::PbftManagerBlockValidationFactStatus::NotChecked,
            reward_votes_status:
                crate::pbft_manager::PbftManagerBlockValidationFactStatus::NotChecked,
            pillar_block_status:
                crate::pbft_manager::PbftManagerBlockValidationFactStatus::NotChecked,
            dag_order_status: crate::pbft_manager::PbftManagerBlockValidationFactStatus::NotChecked,
            dag_weight_status:
                crate::pbft_manager::PbftManagerBlockValidationFactStatus::NotChecked,
        },
        previous_pbft_block_hash: link.prev_block_hash,
        candidate_final_chain_hash: block.val_at(3)?,
        expected_order_hash: block.val_at(2)?,
        pbft_gas_limit: request.pbft_gas_limit,
        reward_vote_hashes,
        pillar_block_hash,
    })
}

fn final_chain_nonce_as_u256(nonce: &rustaxa_types::FinalChainNonce) -> Result<U256> {
    let bytes = nonce.to_bytes();
    anyhow::ensure!(
        bytes.len() <= 32,
        "PBFT_SERVICE_SLASHING_ACCOUNT_NONCE_EXCEEDS_U256"
    );
    Ok(U256::from_big_endian(&bytes))
}

fn resolve_slashing_submitter_facts(
    final_chain: &FinalChain,
    identities: &[SlashingSubmitterIdentity],
) -> Result<Vec<SlashingSubmitterFact>> {
    resolve_slashing_submitter_facts_with(identities, |identity| {
        let account = final_chain.account(identity.address)?;
        Ok(match account {
            Some(account) => (
                final_chain_nonce_as_u256(&account.nonce)?,
                *account.balance.as_u256(),
            ),
            None => (U256::zero(), U256::zero()),
        })
    })
}

fn resolve_slashing_submitter_facts_with(
    identities: &[SlashingSubmitterIdentity],
    mut resolve_account: impl FnMut(&SlashingSubmitterIdentity) -> Result<(U256, U256)>,
) -> Result<Vec<SlashingSubmitterFact>> {
    let mut submitters = Vec::new();
    for identity in identities {
        let (nonce, balance) = resolve_account(identity)?;
        submitters.push(SlashingSubmitterFact {
            wallet_index: identity.wallet_index,
            nonce,
            balance,
        });
        if !balance.is_zero() {
            break;
        }
    }
    Ok(submitters)
}

/// Validated immutable configuration for native PBFT service restoration.
///
/// Millisecond values constrained by the legacy manager runtime are already
/// narrowed to `u32` by the external adapter. Construction derives the current
/// period and Cacti activation from the restored PBFT-chain head; callers
/// cannot inject either fact. Ficus activation and pillar interval configure
/// the immutable pillar request schedule. PBFT sync limits and retained-history
/// facts configure native response service; the sync level must be nonzero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PbftServiceConfig {
    pub genesis_lambda_ms: u32,
    pub cacti_lambda_max_ms: u32,
    pub cacti_lambda_default_ms: u32,
    pub cacti_block: u64,
    pub max_exponential_lambda_ms: u64,
    pub max_steps: u64,
    pub deadline_ms: u64,
    pub polling_interval_ms: u64,
    pub report_malicious_behaviour: bool,
    pub magnolia_activation_period: u64,
    /// First PBFT period where the Ficus pillar schedule is active.
    pub ficus_activation_period: u64,
    /// Number of PBFT periods between pillar blocks; must exceed one.
    pub pillar_blocks_interval: u64,
    /// Maximum finalized periods served by one PBFT sync request.
    pub sync_level_size: u64,
    /// Whether this node retains only bounded finalized history.
    pub is_light_node: bool,
    /// Number of finalized PBFT periods retained by a light node.
    pub light_node_history: u64,
    /// PBFT committee size used by strict sync-certificate validation.
    pub committee_size: u64,
    /// Proposal committee size used by strict sync-certificate validation.
    pub number_of_proposers: u64,
}

/// Native dynamic-lambda decision composed with its durable prior-lambda fact.
///
/// `plan` contains the deterministic finalization policy decision. The lookup
/// is populated only for an accepted, active dynamic-lambda plan; inactive or
/// rejected plans carry `found = false` and value zero without reading storage.
pub struct PbftFinalizationDynamicLambdaDecision {
    /// Deterministic dynamic-lambda policy result.
    pub plan: PbftDynamicLambdaPlan,
    /// Closest persisted lambda at or before the preceding finalized period.
    pub last_saved_period_lambda: PbftFinalizationPeriodLambdaLookup,
}

/// Finalization intent facts with chain-derived fields supplied by `PbftService`.
///
/// This struct intentionally excludes chain-last hashes/period and legacy head
/// payload bytes; the service derives those values from `PbftChainService` state
/// to avoid C++ duplication of chain-read/write invariants.
#[derive(Debug, Clone)]
pub struct PbftFinalizationIntent {
    /// PBFT candidate block hash.
    pub block_hash: H256,
    /// Candidate PBFT period.
    pub block_period: u64,
    /// Candidate PBFT previous hash.
    pub block_prev_hash: H256,
    /// PBFT block pivot DAG anchor hash.
    pub pivot_dag_anchor_hash: H256,
    /// Candidate carries a finalization-required pillar block.
    pub has_pillar_block: bool,
    /// Candidate pillar block already finalized.
    pub pillar_block_finalized: bool,
    /// C++ precomputed dynamic-lambda path requirement.
    pub request_dynamic_lambda_update: bool,
    /// Number of certified votes supplied for this non-duplicate finalization.
    pub cert_vote_count: u64,
    /// Sample certified-vote block hash.
    pub sample_cert_vote_block_hash: H256,
    /// Sample certified-vote period.
    pub sample_cert_vote_period: u64,
    /// Sample certified-vote round.
    pub sample_cert_vote_round: u64,
    /// Sample certified-vote step.
    pub sample_cert_vote_step: u64,
    /// Candidate block Lambda.
    pub block_lambda: u32,
    /// Whether persisted previous-period lambda was found.
    pub last_saved_period_lambda_found: bool,
    /// Previous period lambda value.
    pub last_saved_period_lambda: u32,
    /// Dynamic-lambda blocks-per-year value.
    pub dynamic_blocks_per_year: u32,
    /// Dynamic-lambda post-adjust round counter.
    pub rounds_count_dynamic_lambda: u32,
    /// Dynamic-lambda post-adjust lambda.
    pub dynamic_lambda: u32,
    /// Genesis-configured blocks-per-year.
    pub dpos_blocks_per_year: u32,
    /// Canonical period-data RLP for this candidate.
    pub period_data_rlp: Vec<u8>,
    /// Ordered finalized DAG block hashes.
    pub ordered_dag_block_hashes: Vec<H256>,
    /// Ordered finalized transaction hashes.
    pub ordered_transaction_hashes: Vec<H256>,
    /// Whether to process a pillar block after period advance.
    pub process_pillar_block_after_advance: bool,
}

/// Native result of one planned, durably committed PBFT lifecycle transition.
///
/// `status` and `snapshot` describe the authoritative native commit. The
/// Boolean fields are external executor effects that C++ may apply only when
/// `status` is `Applied`; rejected outcomes clear every effect and preserve the
/// pre-transition snapshot. Planning, own-vote cleanup, storage commit, and
/// runtime publication occur under the manager serialization domain. Storage
/// failures return an error before runtime publication.
pub struct PbftManagerLifecycleTransitionOutcome {
    /// Durable commit status; rejected outcomes publish no runtime mutation.
    pub status: PbftManagerTransitionStorageStatus,
    /// Authoritative manager snapshot after acceptance or before rejection.
    pub snapshot: PbftManagerRuntimeSnapshot,
    /// Drop the matching live C++ cert-voted block sidecar.
    pub remove_cert_voted_sidecar: bool,
    /// Clear live C++ broadcasted-vote sidecars.
    pub clear_broadcasted_vote_sidecars: bool,
    /// Reset the external current-round timer.
    pub reset_current_round_timer: bool,
    /// Reset the external second-finish polling timer.
    pub reset_second_finish_timer: bool,
    /// Emit temporary certify-step compatibility diagnostics.
    pub print_cert_step_info: bool,
    /// Emit temporary second-finish compatibility diagnostics.
    pub print_second_finish_step_info: bool,
    /// Run the delayed executed-block external follow-up.
    pub reset_executed_block_follow_up: bool,
    /// Stable rejection detail; empty after an applied transition.
    pub error_code: String,
}

/// Native result of a manager storage write that reports rejection as data.
///
/// The snapshot is captured under the same manager lock as the durable write.
/// Applied outcomes publish runtime state only after storage succeeds;
/// rejected outcomes preserve the previous snapshot and carry a stable error.
pub struct PbftManagerRuntimeStorageApplyOutcome {
    /// Durable write status.
    pub status: PbftManagerTransitionStorageStatus,
    /// Number of accepted manager storage writes.
    pub applied_writes: u64,
    /// Authoritative post-apply or preserved pre-rejection snapshot.
    pub snapshot: PbftManagerRuntimeSnapshot,
    /// Stable rejection detail; empty after success.
    pub error_code: String,
}

/// Lock-coherent application status used by network and query gating.
///
/// The manager cursor, PBFT-chain head, sync queue, and derived syncing period are
/// sampled under one native serialization guard. The snapshot intentionally
/// excludes DPoS and other external facts so consumers cannot treat it as a
/// service locator or a complete consensus-state projection.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftApplicationStatusSnapshot {
    /// Current native PBFT period.
    pub period: u64,
    /// Current native PBFT round.
    pub round: u64,
    /// Current native PBFT step.
    pub step: u64,
    /// Number of finalized PBFT periods in the native chain.
    pub finalized_chain_size: u64,
    /// Maximum of the queued sync period and native PBFT-chain size.
    pub syncing_period: u64,
    /// Number of period-data entries retained by the native sync queue.
    pub sync_queue_size: u64,
}

/// Complete native result of validation-backed PBFT vote admission.
///
/// Purpose:
/// - Publish the transactional vote-admission result together with the optional
///   Rust-owned slashing transaction effect for one composed operation.
///
/// Invariants and edge behavior:
/// - `slashing_transaction_effect` is present only for a published
///   duplicate-voter conflict with retained canonical payloads.
/// - The effect contains no signing key and no raw vote evidence; signing and
///   transaction insertion remain external leaf operations.
/// - Persistence rejection or an unpublished transition never exposes an
///   effect. FinalChain account lookup errors propagate after admission has
///   already committed, matching the external-effect ordering.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteAdmissionWithSlashingResult {
    /// Terminal canonical vote validation.
    pub validation: PbftCanonicalVoteValidation,
    /// Transactional runtime/persistence publication result.
    pub transaction: PbftVoteAdmissionTransactionResult,
    /// Typed slashing transaction effect for a published conflict, when any.
    pub slashing_transaction_effect: Option<SlashingTransactionEffect>,
}

/// Stable action returned by a native PBFT-sync ingress session.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PbftSyncIngressAction {
    EnqueuedContinue = 0,
    Duplicate = 1,
    SyncComplete = 2,
    Drop = 3,
    StopSyncing = 4,
    Malicious = 5,
    QueueRejected = 6,
    AwaitingSlashing = 7,
}

impl PbftSyncIngressAction {
    /// Returns the stable CXX action code.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Terminal or resumable result of one native PBFT-sync packet ingress session.
///
/// Packet facts are decoded from the exact wire payload and remain immutable
/// while sequential certificate admission pauses for an executable slashing
/// transaction. `AwaitingSlashing` is the only resumable action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftSyncIngressStep {
    pub action: PbftSyncIngressAction,
    pub error_code: String,
    pub source_payload_id: u64,
    pub block_hash: H256,
    pub period: u64,
    pub max_dag_level: u64,
    pub last_block: bool,
    pub current_cert_present: bool,
    pub slashing_transaction_effect: Option<SlashingTransactionEffect>,
}

/// Native current-certificate admission session action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PbftSyncCertBundleAction {
    /// One external signing and transaction-insertion effect must be reported.
    AwaitingSlashing = 0,
    /// All votes were admitted and the native weight threshold was satisfied.
    Accepted = 1,
    /// Shape, vote, persistence, or threshold validation rejected the bundle.
    Rejected = 2,
}

impl PbftSyncCertBundleAction {
    /// Returns the stable CXX action code.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// One resumable step of native current-certificate admission.
///
/// `AwaitingSlashing` carries exactly one effect and no authoritative weighted
/// bundle. `Accepted` is the only action that carries weighted vote payloads.
/// `session_id` and `effect_id` make external reports retry- and mismatch-safe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftSyncCertBundleStep {
    pub action: PbftSyncCertBundleAction,
    pub session_id: u64,
    pub effect_id: u64,
    pub status: PbftSyncCertVoteBundleStatus,
    pub total_weight: u64,
    pub two_t_plus_one: u64,
    pub first_bad_vote_hash: H256,
    pub error_code: String,
    pub weighted_vote_rlps: Vec<Vec<u8>>,
    pub slashing_transaction_effect: Option<SlashingTransactionEffect>,
}

fn sync_cert_bundle_error_code(status: PbftSyncCertVoteBundleStatus) -> &'static str {
    match status {
        PbftSyncCertVoteBundleStatus::Accepted => "",
        PbftSyncCertVoteBundleStatus::Empty => "PBFT_SYNC_CERT_BUNDLE_EMPTY",
        PbftSyncCertVoteBundleStatus::PeriodMismatch => "PBFT_SYNC_CERT_BUNDLE_PERIOD_MISMATCH",
        PbftSyncCertVoteBundleStatus::RoundMismatch => "PBFT_SYNC_CERT_BUNDLE_ROUND_MISMATCH",
        PbftSyncCertVoteBundleStatus::VoteTypeMismatch => {
            "PBFT_SYNC_CERT_BUNDLE_VOTE_TYPE_MISMATCH"
        }
        PbftSyncCertVoteBundleStatus::StepMismatch => "PBFT_SYNC_CERT_BUNDLE_STEP_MISMATCH",
        PbftSyncCertVoteBundleStatus::BlockHashMismatch => {
            "PBFT_SYNC_CERT_BUNDLE_BLOCK_HASH_MISMATCH"
        }
        PbftSyncCertVoteBundleStatus::LiveVoteInvalid => "PBFT_SYNC_CERT_BUNDLE_LIVE_VOTE_INVALID",
        PbftSyncCertVoteBundleStatus::MissingWeight => "PBFT_SYNC_CERT_BUNDLE_MISSING_WEIGHT",
        PbftSyncCertVoteBundleStatus::ThresholdMissing => "PBFT_SYNC_CERT_BUNDLE_THRESHOLD_MISSING",
        PbftSyncCertVoteBundleStatus::InsufficientWeight => {
            "PBFT_SYNC_CERT_BUNDLE_INSUFFICIENT_WEIGHT"
        }
    }
}

struct PbftSyncIngressSession {
    packet: DecodedPbftSyncPacketPrecheck,
    source_payload_id: u64,
    source_peer_id: [u8; 64],
    next_vote: usize,
    slashing_submitters: Vec<SlashingSubmitterIdentity>,
    pending_slashing: Option<SlashingTransactionEffect>,
}

struct PbftSyncCertBundleSession {
    session_id: u64,
    block_period: u64,
    block_hash: H256,
    canonical_vote_rlps: Vec<Vec<u8>>,
    validations: Vec<PbftCanonicalVoteValidation>,
    prepared_weighted_vote_rlps: Vec<Option<Vec<u8>>>,
    threshold: Option<u64>,
    manager_period: u64,
    manager_round: u64,
    submitters: Vec<SlashingSubmitterFact>,
    next_vote: usize,
    next_effect_id: u64,
    pending_slashing: Option<(u64, SlashingTransactionEffect)>,
    weighted_vote_rlps: Vec<Vec<u8>>,
    weighted_facts: Vec<PbftSyncCertVoteFact>,
    total_weight: u64,
}

#[derive(Clone)]
struct PbftSyncCertBundleReportAck {
    session_id: u64,
    effect_id: u64,
    proof_hash: H256,
    transaction_inserted: bool,
    step: Option<PbftSyncCertBundleStep>,
}

struct PbftSyncCertBundleRuntime {
    next_session_id: u64,
    active: Option<PbftSyncCertBundleSession>,
    last_report: Option<PbftSyncCertBundleReportAck>,
}

/// Complete native input for ordinary PBFT block validation composition.
///
/// `fact` carries immutable candidate shape. The remaining fields provide the
/// concrete inputs for the PBFT-chain, FinalChain, reward-vote, pillar, and DAG
/// siblings. Every dependency status and deterministic DAG branch condition is
/// normalized and recomputed on each call. Ordered reward hashes preserve PBFT
/// block order.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftBlockValidationCandidate {
    /// Candidate shape; dependency statuses are ignored and recomputed.
    pub fact: crate::pbft_manager::PbftManagerBlockValidationFact,
    /// Candidate's previous PBFT block hash.
    pub previous_pbft_block_hash: H256,
    /// Candidate's embedded delayed FinalChain hash.
    pub candidate_final_chain_hash: H256,
    /// Expected ordered DAG hash for `prepare_candidate_dag`.
    pub expected_order_hash: H256,
    /// Gas-limit threshold for PBFT divergency weight validation.
    pub pbft_gas_limit: u64,
    /// Candidate reward-vote hashes in canonical PBFT block order.
    pub reward_vote_hashes: Vec<H256>,
    /// Candidate pillar anchor hash when the active rules require one.
    pub pillar_block_hash: Option<H256>,
}

/// External policy required to admit one proposed PBFT block.
///
/// The native PBFT root owns proposal lookup, canonical block decoding,
/// dependency validation, and valid-cache mutation. C++ supplies only the
/// period policy that still belongs to the genesis configuration boundary.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftProposedBlockAdmissionRequest {
    /// Proposed-block index period and expected canonical block period.
    pub period: u64,
    /// Proposed-block index hash and expected canonical signed-block hash.
    pub block_hash: H256,
    /// Gas limit used by candidate DAG divergency validation.
    pub pbft_gas_limit: u64,
    /// Whether the active hardfork requires a decodable extra-data payload.
    pub extra_data_required: bool,
    /// Whether the active schedule requires a pillar-block anchor.
    pub pillar_block_required: bool,
}

/// Terminal status for native proposed-block admission.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftProposedBlockAdmissionStatus {
    /// No proposal exists for the requested period and hash.
    Missing,
    /// The native proposal cache already records successful validation.
    AcceptedAlreadyValid,
    /// Native validation succeeded and the cache was marked valid.
    AcceptedNewlyValidated,
    /// Native validation produced a deterministic rejection or wait result.
    Rejected,
}

impl PbftProposedBlockAdmissionStatus {
    /// Returns the stable bridge representation of this terminal status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Missing => 0,
            Self::AcceptedAlreadyValid => 1,
            Self::AcceptedNewlyValidated => 2,
            Self::Rejected => 3,
        }
    }
}

/// Terminal native proposed-block admission result.
///
/// Accepted results preserve the canonical signed block bytes for the retained
/// C++ vote/executor materialization boundary. Missing and rejected results
/// carry no payload. Decode, storage, and lock failures are returned as errors.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftProposedBlockAdmissionResult {
    /// Terminal admission classification.
    pub status: PbftProposedBlockAdmissionStatus,
    /// Canonical signed block RLP, present only for accepted results.
    pub block_rlp: Vec<u8>,
    /// Stable diagnostic code for logs and bridge consumers.
    pub error_code: &'static str,
}

/// Canonical bytes for one already-signed local PBFT proposal candidate.
///
/// The native task decodes both payloads, proves their shared identity, and
/// validates the block before ranking the proposal vote. The caller retains
/// its live block/vote pair and uses only the returned input index.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftLocalProposalCandidate {
    /// Canonical signed PBFT block RLP.
    pub block_rlp: Vec<u8>,
    /// Canonical signed proposal-vote RLP including its validated weight.
    pub vote_rlp: Vec<u8>,
}

/// Complete policy and payload input for local proposal leader selection.
///
/// The operation owns decoding, PBFT-chain lookup, composed FinalChain/DAG
/// validation, candidate-status derivation, and deterministic ranking. It does
/// not publish proposals or mutate the proposed-block cache; signing and
/// publication remain retained external boundaries.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftLocalProposalSelectionRequest {
    /// Already-signed candidates in caller order.
    pub candidates: Vec<PbftLocalProposalCandidate>,
    /// Expected PBFT period for every block and vote.
    pub period: u64,
    /// Expected PBFT proposal round for every vote.
    pub round: u64,
    /// Gas limit used by candidate DAG divergency validation.
    pub pbft_gas_limit: u64,
    /// Whether the active hardfork requires decodable block extra data.
    pub extra_data_required: bool,
    /// Whether the active schedule requires a pillar-block anchor.
    pub pillar_block_required: bool,
}

/// Terminal result of native local proposal leader selection.
///
/// `selected_index` is meaningful only when `selected` is true and always
/// refers to the unchanged input ordering. Empty or ineligible input is a
/// successful no-selection result; malformed canonical bytes and dependency
/// failures are returned as operation errors.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftLocalProposalSelectionResult {
    /// Whether one candidate was selected.
    pub selected: bool,
    /// Selected input index, or zero when no candidate was selected.
    pub selected_index: u64,
    /// Stable diagnostic code for logs and bridge consumers.
    pub error_code: &'static str,
}

/// CXX-free native owner of the complete PBFT application-service graph.
///
/// Restoration validates storage-independent slashing configuration first,
/// constructs every storage-backed sibling from the same `Arc<Storage>`, and
/// returns only after all siblings have succeeded, preventing publication of a
/// partially initialized root. The chain is restored before manager startup
/// facts are derived. Bootstrap readiness starts pending and is published
/// monotonically through [`Self::complete_bootstrap`].
pub struct PbftService {
    manager: PbftManagerService,
    chain: PbftChainService,
    proposed_blocks: ProposedBlocksService,
    verified_votes: PbftVerifiedVotesService,
    slashing: SlashingProofService,
    readiness: PbftServiceReadiness,
    pillar: PillarChainService,
    network: ConsensusNetworkService,
    sync_ingress: Mutex<Option<PbftSyncIngressSession>>,
    sync_cert_bundle: Mutex<PbftSyncCertBundleRuntime>,
    committee_size: u64,
    number_of_proposers: u64,
    slashing_enabled: bool,
}

impl PbftService {
    const fn empty_replay_outcome() -> PbftVoteRuntimeReplayOutcome {
        PbftVoteRuntimeReplayOutcome {
            should_mark: false,
            inserted: false,
            already_present: false,
        }
    }

    /// Applies one verified-vote slashing transaction insertion report.
    ///
    /// Accepted insertions update the bounded native duplicate cache; rejected
    /// insertions leave the proof retryable. Repeated accepted reports classify
    /// as duplicates and never count as a second submission. Signing and
    /// transaction insertion remain explicit external leaf effects.
    pub fn report_verified_vote_slashing_transaction_submission(
        &self,
        proof_hash: ethereum_types::H256,
        transaction_inserted: bool,
    ) -> Result<DoubleVotingProofSubmissionPlan> {
        self.slashing
            .report_double_voting_proof_submission(proof_hash, transaction_inserted)
    }

    /// Begins native current-certificate admission and advances until either an
    /// external slashing effect or a terminal validation result is reached.
    ///
    /// The complete bundle, threshold lookup, and ordered slashing submitter
    /// account facts are resolved before any vote state is mutated. Only one
    /// session may be active; callers must report every `AwaitingSlashing` step
    /// before Rust admits the next vote.
    pub fn begin_pbft_sync_cert_bundle(
        &self,
        final_chain: &FinalChain,
        block_period: u64,
        block_hash: H256,
        canonical_vote_rlps: Vec<Vec<u8>>,
        slashing_submitters: Vec<SlashingSubmitterIdentity>,
    ) -> Result<PbftSyncCertBundleStep> {
        let mut inspections = Vec::with_capacity(canonical_vote_rlps.len());
        let mut shape_votes = Vec::with_capacity(canonical_vote_rlps.len());
        for vote_rlp in &canonical_vote_rlps {
            let inspection = inspect_canonical_pbft_vote(vote_rlp)?;
            if inspection.status == PbftCanonicalVoteInspectionStatus::MalformedRlp {
                return Ok(Self::sync_cert_terminal_step(
                    0,
                    PbftSyncCertVoteBundleValidation {
                        valid: false,
                        status: PbftSyncCertVoteBundleStatus::LiveVoteInvalid,
                        total_weight: 0,
                        two_t_plus_one: 0,
                        first_bad_vote_hash: inspection.vote_hash,
                    },
                    Vec::new(),
                    Some(inspection.error_code.to_owned()),
                ));
            }
            shape_votes.push(PbftSyncCertVoteFact {
                vote_hash: inspection.vote_hash,
                block_hash: inspection.block_hash,
                period: inspection.period,
                round: inspection.round,
                step: inspection.step,
                vote_type: inspection.vote_type.into(),
                live_vote_valid: true,
                weight_present: inspection.has_embedded_weight,
                weight: inspection.embedded_weight,
            });
            inspections.push(inspection);
        }

        let shape = validate_pbft_sync_cert_vote_bundle(PbftSyncCertVoteBundleFact {
            block_period,
            block_hash,
            votes: shape_votes,
            check_weight_threshold: false,
            two_t_plus_one_found: false,
            two_t_plus_one: 0,
        });
        if !shape.valid {
            return Ok(Self::sync_cert_terminal_step(0, shape, Vec::new(), None));
        }

        let strict_index = if block_period.is_multiple_of(100) {
            None
        } else {
            Some(rand::rng().random_range(0..canonical_vote_rlps.len()))
        };
        let threshold_plan = self
            .verified_votes_two_t_plus_one_threshold_with_final_chain(
                final_chain,
                PbftTwoTPlusOneThresholdFact {
                    pbft_period: block_period.saturating_sub(1),
                    vote_type: PbftVoteType::Cert,
                    current_pbft_chain_size: 0,
                    committee_size: self.committee_size,
                    number_of_proposers: self.number_of_proposers,
                    has_total_dpos_votes_count: false,
                    total_dpos_votes_count: 0,
                    future_dpos_state: false,
                    unknown_error: false,
                },
            )
            .ok();
        let threshold = threshold_plan.as_ref().and_then(|plan| {
            (plan.status == PbftTwoTPlusOneThresholdStatus::Available && plan.has_threshold)
                .then_some(plan.threshold)
        });
        let mut canonical_votes = Vec::with_capacity(canonical_vote_rlps.len());
        let mut validations = Vec::with_capacity(canonical_vote_rlps.len());
        let mut prepared_weighted_vote_rlps = Vec::with_capacity(canonical_vote_rlps.len());
        for (index, (vote_rlp, inspection)) in
            canonical_vote_rlps.into_iter().zip(inspections).enumerate()
        {
            let canonical = if inspection.signature_valid {
                build_slashing_pbft_vote_payload(&vote_rlp)?.vote_rlp
            } else {
                vote_rlp
            };
            let strict_vrf = block_period.is_multiple_of(100) || strict_index == Some(index);
            let (validation, _) = self
                .validate_verified_vote_with_final_chain_internal(
                    final_chain,
                    &canonical,
                    PbftVoteAdmissionValidationRequest {
                        strict_vrf,
                        committee_size: self.committee_size,
                        number_of_proposers: self.number_of_proposers,
                        has_preverified_weight: false,
                        preverified_weight: 0,
                    },
                    false,
                )
                .context("PBFT_SYNC_CERT_VOTE_VALIDATION")?;
            let prepared_weighted = (validation.accepted && validation.weight_calculated)
                .then(|| {
                    build_weighted_pbft_vote_payload(&canonical, validation.calculated_weight)
                        .map(|payload| payload.vote_rlp)
                })
                .transpose()?;
            canonical_votes.push(canonical);
            validations.push(validation);
            prepared_weighted_vote_rlps.push(prepared_weighted);
        }
        let submitters = if self.slashing_enabled {
            slashing_submitters
                .into_iter()
                .map(|identity| SlashingSubmitterFact {
                    wallet_index: identity.wallet_index,
                    nonce: identity.nonce,
                    balance: identity.balance,
                })
                .collect()
        } else {
            Vec::new()
        };
        let manager = self.manager_snapshot();

        let session_id = {
            let mut runtime = self
                .sync_cert_bundle
                .lock()
                .map_err(|_| anyhow::anyhow!("PBFT_SYNC_CERT_BUNDLE_LOCK_POISONED"))?;
            anyhow::ensure!(
                runtime.active.is_none(),
                "PBFT_SYNC_CERT_BUNDLE_ALREADY_ACTIVE"
            );
            let session_id = runtime.next_session_id;
            runtime.next_session_id = runtime.next_session_id.wrapping_add(1).max(1);
            runtime.last_report = None;
            runtime.active = Some(PbftSyncCertBundleSession {
                session_id,
                block_period,
                block_hash,
                canonical_vote_rlps: canonical_votes,
                validations,
                prepared_weighted_vote_rlps,
                threshold,
                manager_period: manager.period,
                manager_round: manager.round,
                submitters,
                next_vote: 0,
                next_effect_id: 1,
                pending_slashing: None,
                weighted_vote_rlps: Vec::new(),
                weighted_facts: Vec::new(),
                total_weight: 0,
            });
            session_id
        };
        let step = match self.advance_pbft_sync_cert_bundle() {
            Ok(step) => step,
            Err(error) => {
                self.abort_pbft_sync_cert_bundle(session_id)?;
                return Err(error);
            }
        };
        debug_assert_eq!(step.session_id, session_id);
        Ok(step)
    }

    /// Reports the exact external slashing effect requested by a current-cert
    /// session and resumes admission. Duplicate identical reports return the
    /// cached next step; stale or mismatched reports leave the pending effect
    /// unchanged.
    pub fn report_pbft_sync_cert_bundle_slashing(
        &self,
        session_id: u64,
        effect_id: u64,
        proof_hash: H256,
        transaction_inserted: bool,
    ) -> Result<PbftSyncCertBundleStep> {
        let mut runtime = self
            .sync_cert_bundle
            .lock()
            .map_err(|_| anyhow::anyhow!("PBFT_SYNC_CERT_BUNDLE_LOCK_POISONED"))?;
        if let Some(ack) = runtime.last_report.as_ref()
            && ack.session_id == session_id
            && ack.effect_id == effect_id
            && ack.proof_hash == proof_hash
            && ack.transaction_inserted == transaction_inserted
        {
            if let Some(step) = ack.step.as_ref() {
                return Ok(step.clone());
            }
            drop(runtime);
            return self.advance_and_cache_pbft_sync_cert_bundle_report(
                session_id,
                effect_id,
                proof_hash,
                transaction_inserted,
            );
        }
        let mut session = runtime
            .active
            .take()
            .ok_or_else(|| anyhow::anyhow!("PBFT_SYNC_CERT_BUNDLE_REPORT_MISMATCH"))?;
        let matches = session.session_id == session_id
            && session
                .pending_slashing
                .as_ref()
                .is_some_and(|(pending_id, effect)| {
                    *pending_id == effect_id && effect.proof_hash == proof_hash
                });
        if !matches {
            runtime.active = Some(session);
            anyhow::bail!("PBFT_SYNC_CERT_BUNDLE_REPORT_MISMATCH");
        }
        if let Err(error) = self
            .report_verified_vote_slashing_transaction_submission(proof_hash, transaction_inserted)
        {
            runtime.active = Some(session);
            return Err(error);
        }
        session.pending_slashing = None;
        runtime.active = Some(session);
        runtime.last_report = Some(PbftSyncCertBundleReportAck {
            session_id,
            effect_id,
            proof_hash,
            transaction_inserted,
            step: None,
        });
        drop(runtime);
        self.advance_and_cache_pbft_sync_cert_bundle_report(
            session_id,
            effect_id,
            proof_hash,
            transaction_inserted,
        )
    }

    /// Clears only the named active current-certificate session.
    ///
    /// This is the exception-unwind boundary for the external signing and
    /// transaction executor. A stale session identifier is a no-op and cannot
    /// clear a newer admission attempt.
    pub fn abort_pbft_sync_cert_bundle(&self, session_id: u64) -> Result<bool> {
        let mut runtime = self
            .sync_cert_bundle
            .lock()
            .map_err(|_| anyhow::anyhow!("PBFT_SYNC_CERT_BUNDLE_LOCK_POISONED"))?;
        let matches = runtime
            .active
            .as_ref()
            .is_some_and(|session| session.session_id == session_id);
        if matches {
            runtime.active = None;
        }
        if runtime
            .last_report
            .as_ref()
            .is_some_and(|ack| ack.session_id == session_id)
        {
            runtime.last_report = None;
        }
        Ok(matches)
    }

    fn advance_and_cache_pbft_sync_cert_bundle_report(
        &self,
        session_id: u64,
        effect_id: u64,
        proof_hash: H256,
        transaction_inserted: bool,
    ) -> Result<PbftSyncCertBundleStep> {
        let step = self.advance_pbft_sync_cert_bundle()?;
        let mut runtime = self
            .sync_cert_bundle
            .lock()
            .map_err(|_| anyhow::anyhow!("PBFT_SYNC_CERT_BUNDLE_LOCK_POISONED"))?;
        let ack = runtime
            .last_report
            .as_mut()
            .filter(|ack| {
                ack.session_id == session_id
                    && ack.effect_id == effect_id
                    && ack.proof_hash == proof_hash
                    && ack.transaction_inserted == transaction_inserted
            })
            .ok_or_else(|| anyhow::anyhow!("PBFT_SYNC_CERT_BUNDLE_REPORT_ACK_LOST"))?;
        ack.step = Some(step.clone());
        Ok(step)
    }

    fn sync_cert_terminal_step(
        session_id: u64,
        validation: PbftSyncCertVoteBundleValidation,
        weighted_vote_rlps: Vec<Vec<u8>>,
        error_code: Option<String>,
    ) -> PbftSyncCertBundleStep {
        PbftSyncCertBundleStep {
            action: if validation.valid {
                PbftSyncCertBundleAction::Accepted
            } else {
                PbftSyncCertBundleAction::Rejected
            },
            session_id,
            effect_id: 0,
            status: validation.status,
            total_weight: validation.total_weight,
            two_t_plus_one: validation.two_t_plus_one,
            first_bad_vote_hash: validation.first_bad_vote_hash,
            error_code: error_code
                .unwrap_or_else(|| sync_cert_bundle_error_code(validation.status).to_owned()),
            weighted_vote_rlps: if validation.valid {
                weighted_vote_rlps
            } else {
                Vec::new()
            },
            slashing_transaction_effect: None,
        }
    }

    fn advance_pbft_sync_cert_bundle(&self) -> Result<PbftSyncCertBundleStep> {
        let mut runtime = self
            .sync_cert_bundle
            .lock()
            .map_err(|_| anyhow::anyhow!("PBFT_SYNC_CERT_BUNDLE_LOCK_POISONED"))?;
        let mut session = runtime
            .active
            .take()
            .ok_or_else(|| anyhow::anyhow!("PBFT_SYNC_CERT_BUNDLE_NO_SESSION"))?;
        if let Some((effect_id, effect)) = session.pending_slashing.as_ref() {
            let step = PbftSyncCertBundleStep {
                action: PbftSyncCertBundleAction::AwaitingSlashing,
                session_id: session.session_id,
                effect_id: *effect_id,
                status: PbftSyncCertVoteBundleStatus::Accepted,
                total_weight: session.total_weight,
                two_t_plus_one: session.threshold.unwrap_or(0),
                first_bad_vote_hash: H256::zero(),
                error_code: String::new(),
                weighted_vote_rlps: Vec::new(),
                slashing_transaction_effect: Some(effect.clone()),
            };
            runtime.active = Some(session);
            return Ok(step);
        }

        while session.next_vote < session.canonical_vote_rlps.len() {
            let index = session.next_vote;
            let canonical = session.canonical_vote_rlps[index].clone();
            let validation = session.validations[index].clone();
            let result = match self.admit_validated_vote_with_slashing_resolver(
                &canonical,
                &validation,
                PbftVoteEventFactFlags {
                    vote_already_known: false,
                    carries_proposed_block: true,
                    valid_stale_reward_vote: false,
                },
                PbftVoteProgressContext {
                    current_period: session.manager_period,
                    current_round: session.manager_round,
                    max_future_period_delta: u64::MAX,
                    two_t_plus_one_threshold: session.threshold,
                    require_proposed_block_sidecar: false,
                    slashing_enabled: true,
                },
                None,
                || Ok(session.submitters.clone()),
            ) {
                Ok(result) => result,
                Err(error) => {
                    runtime.active = Some(session);
                    return Err(error);
                }
            };
            if result.transaction.persistence_status == PbftVoteAdmissionPersistenceStatus::Rejected
            {
                let step = Self::sync_cert_terminal_step(
                    session.session_id,
                    PbftSyncCertVoteBundleValidation {
                        valid: false,
                        status: PbftSyncCertVoteBundleStatus::LiveVoteInvalid,
                        total_weight: session.total_weight,
                        two_t_plus_one: session.threshold.unwrap_or(0),
                        first_bad_vote_hash: result.validation.vote_hash,
                    },
                    Vec::new(),
                    Some(format!(
                        "PBFT_SYNC_CERT_BUNDLE_PERSISTENCE_REJECTED: {}",
                        result.transaction.persistence_error_code
                    )),
                );
                return Ok(step);
            }
            if !result.validation.accepted || !result.validation.weight_calculated {
                return Ok(Self::sync_cert_terminal_step(
                    session.session_id,
                    PbftSyncCertVoteBundleValidation {
                        valid: false,
                        status: PbftSyncCertVoteBundleStatus::LiveVoteInvalid,
                        total_weight: session.total_weight,
                        two_t_plus_one: session.threshold.unwrap_or(0),
                        first_bad_vote_hash: result.validation.vote_hash,
                    },
                    Vec::new(),
                    Some(result.validation.error_code.to_owned()),
                ));
            }

            let weight = result.validation.calculated_weight;
            let weighted = session.prepared_weighted_vote_rlps[index]
                .clone()
                .expect("accepted preflight validation must prepare weighted vote bytes");
            session.total_weight = session.total_weight.saturating_add(weight);
            session.weighted_facts.push(PbftSyncCertVoteFact {
                vote_hash: result.validation.vote_hash,
                block_hash: result.validation.block_hash,
                period: result.validation.period,
                round: result.validation.round,
                step: result.validation.step,
                vote_type: result.validation.vote_type.into(),
                live_vote_valid: true,
                weight_present: true,
                weight,
            });
            session.weighted_vote_rlps.push(weighted);
            session.next_vote += 1;

            if let Some(effect) = result
                .slashing_transaction_effect
                .filter(|effect| effect.status.as_u8() == 0)
            {
                let effect_id = session.next_effect_id;
                session.next_effect_id = session.next_effect_id.wrapping_add(1).max(1);
                session.pending_slashing = Some((effect_id, effect.clone()));
                let step = PbftSyncCertBundleStep {
                    action: PbftSyncCertBundleAction::AwaitingSlashing,
                    session_id: session.session_id,
                    effect_id,
                    status: PbftSyncCertVoteBundleStatus::Accepted,
                    total_weight: session.total_weight,
                    two_t_plus_one: session.threshold.unwrap_or(0),
                    first_bad_vote_hash: H256::zero(),
                    error_code: String::new(),
                    weighted_vote_rlps: Vec::new(),
                    slashing_transaction_effect: Some(effect),
                };
                runtime.active = Some(session);
                return Ok(step);
            }
        }

        let validation = validate_pbft_sync_cert_vote_bundle(PbftSyncCertVoteBundleFact {
            block_period: session.block_period,
            block_hash: session.block_hash,
            votes: std::mem::take(&mut session.weighted_facts),
            check_weight_threshold: true,
            two_t_plus_one_found: session.threshold.is_some(),
            two_t_plus_one: session.threshold.unwrap_or(0),
        });
        Ok(Self::sync_cert_terminal_step(
            session.session_id,
            validation,
            std::mem::take(&mut session.weighted_vote_rlps),
            None,
        ))
    }

    /// Begins or replaces the application-owned PBFT-sync ingress session.
    ///
    /// Native prechecks run before certificate signature recovery and preserve
    /// duplicate/drop precedence. A non-empty native queue bypasses weighted
    /// admission and enqueues the decoded exact child/current-certificate
    /// bytes. An empty queue admits previous-certificate votes sequentially;
    /// the returned step pauses only for an executable slashing transaction.
    pub fn begin_pbft_sync_ingress(
        &self,
        final_chain: &FinalChain,
        packet_rlp: &[u8],
        source_payload_id: u64,
        source_peer_id: [u8; 64],
        slashing_submitters: Vec<SlashingSubmitterIdentity>,
    ) -> Result<PbftSyncIngressStep> {
        *self
            .sync_ingress
            .lock()
            .map_err(|_| anyhow::anyhow!("PBFT_SYNC_INGRESS_LOCK_POISONED"))? = None;
        let decision = self
            .network
            .precheck_pbft_sync_packet(packet_rlp, source_payload_id)?;
        let decoded = decode_pbft_sync_packet_precheck(packet_rlp).ok();
        if decision.status != NETWORK_INGRESS_STATUS_ACCEPTED {
            let action = match decision.status {
                NETWORK_INGRESS_STATUS_PBFT_SYNC_DUPLICATE_BLOCK => {
                    PbftSyncIngressAction::Duplicate
                }
                NETWORK_INGRESS_STATUS_PBFT_SYNC_COMPLETE => PbftSyncIngressAction::SyncComplete,
                NETWORK_INGRESS_STATUS_PBFT_SYNC_MALICIOUS => PbftSyncIngressAction::Malicious,
                _ => PbftSyncIngressAction::Drop,
            };
            return Ok(Self::sync_ingress_step(
                decoded.as_ref(),
                source_payload_id,
                action,
                decision.error_code,
                None,
            ));
        }
        let mut packet = decoded.ok_or_else(|| anyhow::anyhow!("PBFT_SYNC_INGRESS_DECODE_LOST"))?;
        packet.period_data.entry.source_peer_id = source_peer_id;
        *self
            .sync_ingress
            .lock()
            .map_err(|_| anyhow::anyhow!("PBFT_SYNC_INGRESS_LOCK_POISONED"))? =
            Some(PbftSyncIngressSession {
                packet,
                source_payload_id,
                source_peer_id,
                next_vote: 0,
                slashing_submitters,
                pending_slashing: None,
            });
        self.advance_pbft_sync_ingress(final_chain)
    }

    /// Reports the pending slashing insertion result and immediately advances.
    ///
    /// The proof hash must equal the current executable effect. Successful
    /// insertion updates duplicate protection; a failed insertion stays
    /// retryable by the slashing planner but does not reorder later certificate
    /// admissions. No FinalChain reference is retained after this call.
    pub fn report_pbft_sync_ingress_slashing(
        &self,
        final_chain: &FinalChain,
        proof_hash: H256,
        transaction_inserted: bool,
    ) -> Result<PbftSyncIngressStep> {
        {
            let mut ingress = self
                .sync_ingress
                .lock()
                .map_err(|_| anyhow::anyhow!("PBFT_SYNC_INGRESS_LOCK_POISONED"))?;
            let session = ingress
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("PBFT_SYNC_INGRESS_NO_SESSION"))?;
            let pending = session
                .pending_slashing
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("PBFT_SYNC_INGRESS_NOT_AWAITING_SLASHING"))?;
            anyhow::ensure!(
                pending.proof_hash == proof_hash,
                "PBFT_SYNC_INGRESS_SLASHING_PROOF_HASH_MISMATCH"
            );
            self.report_verified_vote_slashing_transaction_submission(
                proof_hash,
                transaction_inserted,
            )?;
            session.pending_slashing = None;
        }
        self.advance_pbft_sync_ingress(final_chain)
    }

    fn advance_pbft_sync_ingress(&self, final_chain: &FinalChain) -> Result<PbftSyncIngressStep> {
        let mut ingress = self
            .sync_ingress
            .lock()
            .map_err(|_| anyhow::anyhow!("PBFT_SYNC_INGRESS_LOCK_POISONED"))?;
        let session = ingress
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("PBFT_SYNC_INGRESS_NO_SESSION"))?;
        if session.pending_slashing.is_some() {
            return Err(anyhow::anyhow!(
                "PBFT_SYNC_INGRESS_AWAITING_SLASHING_REPORT"
            ));
        }

        if !self.period_data_queue_snapshot()?.empty {
            let outcome = self.push_sync_ingress_packet(session, None)?;
            let action = if outcome.accepted {
                PbftSyncIngressAction::EnqueuedContinue
            } else {
                PbftSyncIngressAction::QueueRejected
            };
            let step = Self::sync_ingress_step(
                Some(&session.packet),
                session.source_payload_id,
                action,
                if outcome.accepted {
                    String::new()
                } else {
                    "PBFT_SYNC_INGRESS_QUEUE_REJECTED".into()
                },
                None,
            );
            *ingress = None;
            return Ok(step);
        }

        while session.next_vote
            < session
                .packet
                .period_data
                .entry
                .previous_cert_vote_rlps
                .len()
        {
            let vote =
                session.packet.period_data.entry.previous_cert_vote_rlps[session.next_vote].clone();
            let manager = self.manager_snapshot();
            let result = self.admit_and_persist_verified_vote_with_external_slashing_facts(
                final_chain,
                &vote,
                PbftVoteAdmissionValidationRequest {
                    strict_vrf: true,
                    committee_size: self.committee_size,
                    number_of_proposers: self.number_of_proposers,
                    has_preverified_weight: false,
                    preverified_weight: 0,
                },
                PbftVoteEventFactFlags {
                    vote_already_known: false,
                    carries_proposed_block: false,
                    valid_stale_reward_vote: true,
                },
                PbftVoteProgressContext {
                    current_period: manager.period,
                    current_round: manager.round,
                    max_future_period_delta: 0,
                    two_t_plus_one_threshold: None,
                    require_proposed_block_sidecar: false,
                    slashing_enabled: self.slashing_enabled,
                },
                &session.slashing_submitters,
            )?;
            if !result.validation.accepted {
                let step = Self::sync_ingress_step(
                    Some(&session.packet),
                    session.source_payload_id,
                    PbftSyncIngressAction::Malicious,
                    result.validation.error_code.into(),
                    None,
                );
                *ingress = None;
                return Ok(step);
            }
            if result.transaction.persistence_required && !result.transaction.transition_published {
                let step = Self::sync_ingress_step(
                    Some(&session.packet),
                    session.source_payload_id,
                    PbftSyncIngressAction::QueueRejected,
                    result.transaction.persistence_error_code,
                    None,
                );
                *ingress = None;
                return Ok(step);
            }
            session.next_vote += 1;
            if let Some(effect) = result
                .slashing_transaction_effect
                .filter(|effect| effect.status.as_u8() == 0)
            {
                session.pending_slashing = Some(effect.clone());
                return Ok(Self::sync_ingress_step(
                    Some(&session.packet),
                    session.source_payload_id,
                    PbftSyncIngressAction::AwaitingSlashing,
                    String::new(),
                    Some(effect),
                ));
            }
        }

        let block = &session.packet.period_data.entry;
        let rewards =
            self.select_reward_vote_payloads(block.period, block.reward_vote_hashes.clone())?;
        if !rewards.accepted {
            let action = classify_sync_reward_failure(block.period, self.reward_vote_period()?);
            let step = Self::sync_ingress_step(
                Some(&session.packet),
                session.source_payload_id,
                action,
                rewards.status.legacy_error_code().into(),
                None,
            );
            *ingress = None;
            return Ok(step);
        }
        let weighted_votes = rewards
            .selected_records
            .into_iter()
            .map(|record| record.vote_rlp)
            .collect();
        let outcome = self.push_sync_ingress_packet(session, Some(weighted_votes))?;
        let action = if outcome.accepted {
            PbftSyncIngressAction::EnqueuedContinue
        } else {
            PbftSyncIngressAction::QueueRejected
        };
        let step = Self::sync_ingress_step(
            Some(&session.packet),
            session.source_payload_id,
            action,
            if outcome.accepted {
                String::new()
            } else {
                "PBFT_SYNC_INGRESS_QUEUE_REJECTED".into()
            },
            None,
        );
        *ingress = None;
        Ok(step)
    }

    fn push_sync_ingress_packet(
        &self,
        session: &PbftSyncIngressSession,
        weighted_previous_cert_votes: Option<Vec<Vec<u8>>>,
    ) -> Result<PeriodDataQueuePushOutcome> {
        let mut entry = session.packet.period_data.entry.clone();
        entry.source_peer_id = session.source_peer_id;
        if let Some(votes) = weighted_previous_cert_votes {
            entry.previous_cert_vote_rlps = votes;
            entry.previous_cert_first_vote_has_weight = !entry.previous_cert_vote_rlps.is_empty();
        }
        let chain_size = self.pbft_chain_head().size;
        self.push_period_data_queue(PeriodDataQueuePushRequest {
            entry,
            max_pbft_size: chain_size,
            current_block_cert_vote_rlps: session
                .packet
                .period_data
                .current_block_cert_vote_rlps
                .clone(),
        })
    }

    fn sync_ingress_step(
        packet: Option<&DecodedPbftSyncPacketPrecheck>,
        source_payload_id: u64,
        action: PbftSyncIngressAction,
        error_code: String,
        slashing_transaction_effect: Option<SlashingTransactionEffect>,
    ) -> PbftSyncIngressStep {
        PbftSyncIngressStep {
            action,
            error_code,
            source_payload_id,
            block_hash: packet
                .map(|packet| packet.period_data.entry.block_hash)
                .unwrap_or_default(),
            period: packet
                .map(|packet| packet.period_data.entry.period)
                .unwrap_or_default(),
            max_dag_level: packet
                .map(|packet| packet.max_dag_level)
                .unwrap_or_default(),
            last_block: packet.map(|packet| packet.last_block).unwrap_or(false),
            current_cert_present: packet
                .map(|packet| packet.current_cert_votes_present)
                .unwrap_or(false),
            slashing_transaction_effect,
        }
    }
    /// Loads one finalized startup-replay period through manager-owned storage.
    ///
    /// Missing periods remain explicit in the returned native payload. Optional
    /// closest-lambda lookup, row decoding, and storage errors stay native.
    pub fn load_startup_replay_period(
        &self,
        period: u64,
        load_period_lambda: bool,
    ) -> Result<PbftManagerStartupReplayPeriod> {
        let manager = self.manager_state();
        load_pbft_manager_startup_replay_period(
            manager.storage.as_ref(),
            period,
            load_period_lambda,
        )
    }

    /// Loads the local node's persisted pillar vote through manager-owned storage.
    ///
    /// Missing data follows the native storage contract; malformed rows and
    /// operational failures propagate without mutating manager state.
    pub fn own_pillar_block_vote(&self) -> Result<Vec<u8>> {
        let manager = self.manager_state();
        load_own_pillar_block_vote_storage(manager.storage.as_ref())
    }

    /// Resolves a finalized DAG block position through manager-owned storage.
    pub fn dag_block_period(
        &self,
        hash: ethereum_types::H256,
    ) -> Result<DagBlockPeriodStorageLookup> {
        let manager = self.manager_state();
        dag_block_period_from_storage(manager.storage.as_ref(), hash)
    }

    /// Checks finalized PBFT block membership through manager-owned storage.
    ///
    /// This query does not materialize a legacy PBFT block or mutate state.
    pub fn pbft_block_in_db(&self, hash: ethereum_types::H256) -> Result<bool> {
        let manager = self.manager_state();
        pbft_block_exists_in_storage(manager.storage.as_ref(), hash)
    }

    /// Returns an owned snapshot of the native PBFT manager scalar state.
    ///
    /// Snapshot capture occurs under the manager lock and the returned value
    /// remains valid after the serialization domain is released.
    pub fn manager_snapshot(&self) -> PbftManagerRuntimeSnapshot {
        self.manager_state().state.snapshot()
    }

    /// Returns one coherent PBFT application status snapshot.
    ///
    /// All fields are sampled while the manager serialization guard owns
    /// both the scalar runtime and its PBFT-chain/period-queue siblings. The
    /// queue size is widened losslessly for the stable CXX boundary.
    pub fn application_status_snapshot(&self) -> Result<PbftApplicationStatusSnapshot> {
        self.manager_and_application_status_snapshot()
            .map(|(_, status)| status)
    }

    /// Returns the manager runtime and its client-facing live status under one guard.
    pub fn manager_and_application_status_snapshot(
        &self,
    ) -> Result<(PbftManagerRuntimeSnapshot, PbftApplicationStatusSnapshot)> {
        let manager = self.manager_state();
        let runtime = manager.state.snapshot();
        let chain_size = manager.chain.head().size;
        let status = PbftApplicationStatusSnapshot {
            period: runtime.period,
            round: runtime.round,
            step: runtime.step,
            finalized_chain_size: chain_size,
            syncing_period: manager.period_data_queue.syncing_period(chain_size),
            sync_queue_size: u64::try_from(manager.period_data_queue.size())
                .context("PBFT_APPLICATION_STATUS_QUEUE_SIZE_OVERFLOW")?,
        };
        Ok((runtime, status))
    }

    /// Plans the ordered post-reset period-advance effects under the manager lock.
    ///
    /// The plan derives from the last committed native reset provenance and the
    /// supplied PBFT-chain size; it performs no storage or external effects.
    pub fn plan_advance_period_after_reset(
        &self,
        pbft_chain_size: u64,
    ) -> PbftManagerAdvancePeriodPlan {
        self.manager_state()
            .state
            .plan_advance_period_after_reset(pbft_chain_size)
    }

    /// Publishes validated live broadcast counters under the manager lock.
    ///
    /// Zero counters remain rejected by the scalar runtime without mutation.
    /// This operation is process-local and performs no durable write.
    pub fn apply_broadcast_counters(
        &self,
        broadcast_votes_counter: u32,
        rebroadcast_votes_counter: u32,
        broadcast_reward_votes_counter: u32,
        rebroadcast_reward_votes_counter: u32,
    ) -> PbftManagerRuntimeSnapshot {
        self.manager_state()
            .state
            .apply_committed_broadcast_counters(
                broadcast_votes_counter,
                rebroadcast_votes_counter,
                broadcast_reward_votes_counter,
                rebroadcast_reward_votes_counter,
            )
    }

    /// Loads the persisted cert-voted recovery payload through manager-owned storage.
    ///
    /// Missing data is represented by an empty byte vector. Storage failures
    /// propagate and no live manager state is changed.
    pub fn cert_voted_block_in_round(&self) -> Result<Vec<u8>> {
        Ok(self
            .manager_state()
            .storage
            .pbft()
            .cert_voted_block_in_round_rlp()?
            .unwrap_or_default())
    }

    /// Persists a cert-voted recovery payload before publishing its live metadata.
    ///
    /// The storage write and scalar publication share one manager lock epoch;
    /// failures return before the runtime snapshot changes.
    pub fn save_cert_voted_block_in_round(
        &self,
        period: u64,
        round: u32,
        block_hash: ethereum_types::H256,
        block_rlp: &[u8],
    ) -> Result<PbftManagerRuntimeSnapshot> {
        let mut manager = self.manager_state();
        save_cert_voted_block_in_round_storage(
            manager.storage.as_ref(),
            u64::from(round),
            block_rlp,
        )?;
        Ok(manager
            .state
            .apply_committed_cert_voted_block(period, u64::from(round), block_hash))
    }

    /// Publishes already-persisted cert-voted metadata under the manager lock.
    ///
    /// This compatibility task performs no storage write and is used only when
    /// a retained executor has already established durable ownership.
    pub fn apply_cert_voted_block_metadata(
        &self,
        period: u64,
        round: u32,
        block_hash: ethereum_types::H256,
    ) -> PbftManagerRuntimeSnapshot {
        self.manager_state().state.apply_committed_cert_voted_block(
            period,
            u64::from(round),
            block_hash,
        )
    }

    /// Prepares, validates, and caches the canonical DAG payload for a PBFT candidate.
    ///
    /// Cache identity is the anchor alone: a previously accepted anchor
    /// short-circuits without revalidating period, hash, or gas limit. Fresh
    /// preparation enforces live DAG period/current-anchor availability,
    /// canonical order hash, and the legacy previous-PBFT-pivot GHOST `[1]`
    /// divergence weight rule. Failed candidates publish no partial payload.
    pub fn prepare_candidate_dag(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        period: u64,
        anchor: H256,
        expected_order_hash: H256,
        pbft_gas_limit: u64,
    ) -> Result<PbftCandidateDagPreparationStatus> {
        if self
            .manager_state()
            .state
            .has_cached_anchor_dag_order(anchor)
        {
            return Ok(PbftCandidateDagPreparationStatus::Valid);
        }

        let Some(prepared) =
            dag_transaction_service.prepare_pbft_candidate_payload(period, anchor)?
        else {
            return Ok(PbftCandidateDagPreparationStatus::Missing);
        };
        if pbft_candidate_dag_order_hash(&prepared.payload) != expected_order_hash {
            return Ok(PbftCandidateDagPreparationStatus::OrderHashInvalid);
        }

        let chain_head = self.chain.head();
        if chain_head.last_pbft_block_hash != H256::zero() {
            let previous = self.chain.block_rlp(chain_head.last_pbft_block_hash)?;
            anyhow::ensure!(
                previous.found,
                "PBFT_CANDIDATE_DAG_PREVIOUS_PBFT_BLOCK_MISSING"
            );
            let previous = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&previous.block_rlp))
                .context("PBFT_CANDIDATE_DAG_PREVIOUS_PBFT_BLOCK_DECODE")?;
            let ghost = dag_transaction_service.dag_ghost_path(
                crate::dag_transaction_service::DagGhostPathRoot::Block(
                    previous.pivot_dag_block_hash,
                ),
            )?;
            if ghost.len() > 1
                && anchor != ghost[1]
                && prepared.total_gas > U256::from(pbft_gas_limit)
            {
                return Ok(PbftCandidateDagPreparationStatus::WeightInvalid);
            }
        }

        self.manager_state()
            .state
            .cache_candidate_dag_payload(anchor, prepared.payload);
        Ok(PbftCandidateDagPreparationStatus::Valid)
    }

    /// Returns one previously validated candidate payload by anchor.
    ///
    /// The returned payload is owned by the caller. Unknown anchors are an
    /// explicit error because external finalization must never materialize an
    /// empty DAG bundle after candidate validation succeeded.
    pub fn cached_candidate_dag_payload(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        anchor: H256,
    ) -> Result<crate::dag_service::DagRuntimeNonFinalizedSyncPayload> {
        let payload = self
            .manager_state()
            .state
            .cached_candidate_dag_payload(anchor)
            .ok_or_else(|| anyhow::anyhow!("PBFT_CANDIDATE_DAG_CACHE_MISSING"))?;
        dag_transaction_service.hydrate_pbft_candidate_transactions(payload)
    }

    /// Plans the manager's deadline sleep from one lock-consistent snapshot.
    pub fn plan_runtime_sleep_until_next_step(
        &self,
        round_elapsed_ms: i64,
    ) -> PbftManagerSleepPlan {
        plan_pbft_manager_runtime_sleep_until_next_step(
            &self.manager_state().state.snapshot(),
            round_elapsed_ms,
        )
    }

    /// Resets the native PBFT sync queue-drain cursor after bootstrap.
    ///
    /// Calls made before readiness are ignored. The live period-data sidecars
    /// remain external executor inputs, while action ordering and report state
    /// stay within the manager lock domain.
    pub fn begin_pbft_sync_queue_drain(&self) {
        if !self.readiness.is_ready() {
            return;
        }
        let mut manager = self.manager_state();
        manager.pbft_sync_queue_drain_session = create_pbft_sync_queue_drain_session();
    }

    /// Returns the next external queue-drain executor step after bootstrap.
    ///
    /// `None` denotes incomplete bootstrap. Queue size and period come from
    /// manager-owned queue and chain state; cursor advancement and requested
    /// stale cleanup occur under the same manager lock. The infallible native
    /// cleanup step is acknowledged internally and is never exposed as a
    /// fallible C++ executor action.
    pub fn pbft_sync_queue_drain_next(&self) -> Option<PbftSyncQueueDrainStep> {
        if !self.readiness.is_ready() {
            return None;
        }
        let mut manager = self.manager_state();
        loop {
            let queue_size = manager.period_data_queue.size();
            let current_period = manager.chain.head().size.saturating_add(1);
            let step = next_pbft_sync_queue_drain_step(
                &mut manager.pbft_sync_queue_drain_session,
                queue_size,
                current_period,
            );
            if step.action != crate::pbft_sync::PbftSyncQueueDrainAction::CleanOldData {
                return Some(step);
            }

            manager
                .period_data_queue
                .clean_old_data(step.clean_before_period);
            let result = report_pbft_sync_queue_drain_step(
                &mut manager.pbft_sync_queue_drain_session,
                PbftSyncQueueDrainReport {
                    action: crate::pbft_sync::PbftSyncQueueDrainAction::CleanOldData,
                    success: true,
                    accepted_period_data: false,
                },
            );
            debug_assert!(result.can_continue);
        }
    }

    /// Applies one external queue-drain report under the manager lock.
    ///
    /// `None` denotes incomplete bootstrap. Mismatched or failed reports are
    /// returned as native terminal results without exposing the cursor.
    pub fn report_pbft_sync_queue_drain(
        &self,
        report: PbftSyncQueueDrainReport,
    ) -> Option<PbftSyncQueueDrainReportResult> {
        if !self.readiness.is_ready() {
            return None;
        }
        let mut manager = self.manager_state();
        Some(report_pbft_sync_queue_drain_step(
            &mut manager.pbft_sync_queue_drain_session,
            report,
        ))
    }

    /// Starts or replaces the native daemon-tick executor cursor when bootstrap is complete.
    ///
    /// The supplied facts are consumed under the manager lock. Calls made
    /// before bootstrap publication are ignored, preserving the live daemon's
    /// fail-closed startup contract.
    pub fn begin_runtime_session(&self, fact: PbftManagerRuntimeTickFact) {
        if !self.readiness.is_ready() {
            return;
        }
        let mut manager = self.manager_state();
        manager.runtime_session = Some(create_pbft_manager_runtime_session(fact));
    }

    /// Returns the current daemon-tick executor step without advancing it.
    ///
    /// `None` means no cursor has been started. The returned step is owned and
    /// remains valid after the manager lock is released.
    pub fn runtime_session_next(&self) -> Option<PbftManagerRuntimeSessionStep> {
        let manager = self.manager_state();
        manager
            .runtime_session
            .as_ref()
            .map(next_pbft_manager_runtime_action)
    }

    /// Applies one external daemon action report and returns the resulting step.
    ///
    /// The cursor is removed, advanced, and republished in one manager lock
    /// epoch. `None` means no cursor was active; invalid reports remain encoded
    /// in the returned native session step.
    pub fn report_runtime_session(
        &self,
        report: PbftManagerRuntimeActionReport,
    ) -> Option<PbftManagerRuntimeSessionStep> {
        let mut manager = self.manager_state();
        let session = manager.runtime_session.take()?;
        manager.runtime_session = Some(report_pbft_manager_runtime_action(session, report));
        manager
            .runtime_session
            .as_ref()
            .map(next_pbft_manager_runtime_action)
    }

    /// Aborts the active daemon-tick cursor under the manager lock.
    ///
    /// An absent cursor is a no-op. The aborted terminal cursor is retained so
    /// a subsequent query observes the stable native abort error.
    pub fn abort_runtime_session(&self) {
        let mut manager = self.manager_state();
        if let Some(session) = manager.runtime_session.take() {
            manager.runtime_session = Some(abort_pbft_manager_runtime_session(session));
        }
    }

    /// Starts or replaces the native PBFT state-action effect cursor.
    ///
    /// Construction and publication occur in one manager lock epoch. C++ may
    /// execute returned leaf effects but cannot access or mutate the cursor.
    pub fn begin_state_action_effect_session(&self, fact: PbftManagerStateActionFact) {
        let mut manager = self.manager_state();
        manager.state_action_effect_session =
            Some(create_pbft_manager_state_action_effect_session(fact));
    }

    /// Returns and advances the current state-action effect cursor.
    ///
    /// `None` means no cursor is active. Terminal and validation status remain
    /// encoded in the owned native step.
    pub fn state_action_effect_session_next(&self) -> Option<PbftManagerStateActionSessionStep> {
        let mut manager = self.manager_state();
        manager
            .state_action_effect_session
            .as_mut()
            .map(next_pbft_manager_state_action_effect_session)
    }

    /// Applies one external state-action effect report and returns the next step.
    ///
    /// Report validation and cursor mutation share the manager lock. `None`
    /// means no cursor is active; invalid reports are represented by the native
    /// terminal step rather than bridge-owned state.
    pub fn report_state_action_effect_session(
        &self,
        report: PbftManagerStateActionEffectReport,
    ) -> Option<PbftManagerStateActionSessionStep> {
        let mut manager = self.manager_state();
        manager
            .state_action_effect_session
            .as_mut()
            .map(|session| report_pbft_manager_state_action_effect_session(session, report))
    }

    /// Starts or replaces the native PBFT proposal cursor after bootstrap.
    ///
    /// Calls made before bootstrap completion are ignored. Proposal facts and
    /// cursor publication remain inside the native manager serialization
    /// domain; FinalChain, DAG-order, signing, and transport stay leaf effects.
    pub fn begin_proposal_session(&self, fact: PbftManagerProposalInitialFact) {
        if !self.readiness.is_ready() {
            return;
        }
        let mut manager = self.manager_state();
        manager.proposal_session_generation = manager
            .proposal_session_generation
            .checked_add(1)
            .expect("PBFT proposal cursor generation exhausted");
        manager.proposal_session = Some(create_pbft_manager_proposal_session(fact));
    }

    /// Returns and advances the current PBFT proposal cursor after bootstrap.
    ///
    /// `None` is returned before readiness or when no proposal cursor exists.
    /// The owned step remains valid after releasing the manager lock.
    pub fn proposal_session_next(&self) -> Option<PbftManagerProposalSessionStep> {
        if !self.readiness.is_ready() {
            return None;
        }
        let mut manager = self.manager_state();
        manager
            .proposal_session
            .as_mut()
            .map(next_pbft_manager_proposal_session)
    }

    /// Applies one DAG-order leaf report to the native proposal cursor.
    ///
    /// Report validation and cursor advancement occur under one manager lock.
    /// `None` means no cursor is active; all protocol rejection details remain
    /// encoded in the returned native step.
    pub fn report_proposal_dag_order(
        &self,
        report: PbftManagerProposalDagOrderReport,
    ) -> Option<PbftManagerProposalSessionStep> {
        let mut manager = self.manager_state();
        manager
            .proposal_session
            .as_mut()
            .map(|session| report_pbft_manager_proposal_dag_order(session, report))
    }

    /// Advances the active proposal cursor through every native DAG-order request.
    ///
    /// The manager lock is held only while reading or reporting one cursor
    /// step. Canonical DAG block RLP decoding, order selection, and gas-fact
    /// preparation run after releasing it under the DAG/transaction service's
    /// own lock order. Missing order is reported into the cursor as the
    /// existing terminal `MissingDagOrder` status. Storage or decode errors are
    /// returned without reporting, leaving the exact pending anchor retryable.
    /// A concurrently replaced cursor is detected by its generation and returns
    /// a stale-cursor contract step without mutating the replacement.
    pub fn proposal_session_next_with_dag(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
    ) -> Result<Option<PbftManagerProposalSessionStep>> {
        if !self.readiness.is_ready() {
            return Ok(None);
        }
        let Some((mut step, expected_period, expected_generation)) = ({
            let mut manager = self.manager_state();
            let generation = manager.proposal_session_generation;
            manager.proposal_session.as_mut().map(|session| {
                (
                    next_pbft_manager_proposal_session(session),
                    session.fact.period,
                    generation,
                )
            })
        }) else {
            return Ok(None);
        };

        loop {
            if step.action != PbftManagerProposalAction::RequestDagOrder {
                return Ok(Some(step));
            }
            let requested_anchor = step.requested_anchor_hash;
            let prepared = dag_transaction_service
                .prepare_pbft_proposal_blocks(expected_period, requested_anchor)?;
            let report = match prepared {
                Some(prepared) => PbftManagerProposalDagOrderReport {
                    anchor_hash: requested_anchor,
                    dag_blocks: prepared
                        .into_iter()
                        .map(|block| PbftManagerProposalDagBlockFact {
                            hash: block.hash,
                            gas_estimation: block.gas_estimation,
                        })
                        .collect(),
                    order_available: true,
                },
                None => PbftManagerProposalDagOrderReport {
                    anchor_hash: requested_anchor,
                    dag_blocks: Vec::new(),
                    order_available: false,
                },
            };
            let reported =
                self.report_proposal_dag_order_for_generation(expected_generation, report);
            let Some(reported) = reported else {
                return Ok(None);
            };
            step = reported;
        }
    }

    fn report_proposal_dag_order_for_generation(
        &self,
        expected_generation: u64,
        report: PbftManagerProposalDagOrderReport,
    ) -> Option<PbftManagerProposalSessionStep> {
        let mut manager = self.manager_state();
        if manager.proposal_session_generation != expected_generation {
            return Some(stale_pbft_manager_proposal_session_step());
        }
        manager
            .proposal_session
            .as_mut()
            .map(|session| report_pbft_manager_proposal_dag_order(session, report))
    }

    /// Durably clears the delayed executed-block status and then publishes it.
    ///
    /// Storage rejection is returned as a rejected typed outcome because the
    /// retained C++ follow-up treats it as an executor result. The previous
    /// snapshot is preserved and no runtime state is published on failure.
    pub fn apply_executed_block_reset(&self) -> PbftManagerRuntimeStorageApplyOutcome {
        let mut manager = self.manager_state();
        if apply_executed_block_reset_storage(manager.storage.as_ref()).is_err() {
            return PbftManagerRuntimeStorageApplyOutcome {
                status: PbftManagerTransitionStorageStatus::Rejected,
                applied_writes: 0,
                snapshot: manager.state.snapshot(),
                error_code: "PBFT_MANAGER_EXECUTED_BLOCK_RESET_WRITE_FAILURE".to_owned(),
            };
        }
        manager.state.apply_committed_executed_block_reset();
        PbftManagerRuntimeStorageApplyOutcome {
            status: PbftManagerTransitionStorageStatus::Applied,
            applied_writes: 1,
            snapshot: manager.state.snapshot(),
            error_code: String::new(),
        }
    }

    /// Persists and publishes one supported next-voted manager status.
    ///
    /// Unsupported status ids or storage errors return before runtime
    /// publication, preserving the previous snapshot.
    pub fn apply_next_voted_status(&self, status: u8) -> Result<PbftManagerRuntimeSnapshot> {
        let mut manager = self.manager_state();
        apply_next_voted_status_storage(manager.storage.as_ref(), status)?;
        manager.state.apply_committed_next_voted_status(status);
        Ok(manager.state.snapshot())
    }

    /// Persists and publishes one supported manager round/step cursor field.
    ///
    /// Unsupported fields or storage errors return before runtime publication;
    /// dynamic-lambda storage remains owned by finalization.
    pub fn apply_cursor_field(&self, field: u8, value: u32) -> Result<PbftManagerRuntimeSnapshot> {
        let mut manager = self.manager_state();
        apply_pbft_manager_cursor_field_storage(manager.storage.as_ref(), field, value)?;
        manager.state.apply_committed_cursor_field(field, value);
        Ok(manager.state.snapshot())
    }

    fn rejected_lifecycle_transition(
        snapshot: PbftManagerRuntimeSnapshot,
        error_code: String,
    ) -> PbftManagerLifecycleTransitionOutcome {
        PbftManagerLifecycleTransitionOutcome {
            status: PbftManagerTransitionStorageStatus::Rejected,
            snapshot,
            remove_cert_voted_sidecar: false,
            clear_broadcasted_vote_sidecars: false,
            reset_current_round_timer: false,
            reset_second_finish_timer: false,
            print_cert_step_info: false,
            print_second_finish_step_info: false,
            reset_executed_block_follow_up: false,
            error_code,
        }
    }

    /// Plans, persists, and publishes one PBFT lifecycle transition.
    ///
    /// Invalid transition facts return a rejected outcome without storage or
    /// runtime mutation. Ready transitions lock the own-vote family when
    /// required, commit all manager/status/vote writes, then publish the native
    /// cursor and reset provenance. Operational storage errors propagate before
    /// publication. Returned effect flags are the only remaining C++ executor
    /// work and are populated only after a successful native commit.
    pub fn execute_lifecycle_transition(
        &self,
        request: PbftManagerLifecycleTransitionRequest,
    ) -> Result<PbftManagerLifecycleTransitionOutcome> {
        let mut manager = self.manager_state();
        let plan = manager.state.plan_lifecycle_transition(request);
        if plan.status != PbftManagerTransitionStatus::Ready {
            return Ok(Self::rejected_lifecycle_transition(
                manager.state.snapshot(),
                plan.error_code,
            ));
        }

        let own_votes_guard = if plan.clear_own_votes {
            Some(manager.storage.lock_own_verified_votes()?)
        } else {
            None
        };
        let own_vote_hashes = if own_votes_guard.is_some() {
            manager.storage.pbft().own_verified_vote_hashes()?
        } else {
            Vec::new()
        };
        let storage_result = apply_pbft_manager_transition_storage(
            manager.storage.as_ref(),
            &plan,
            &own_vote_hashes,
            false,
        )?;
        drop(own_votes_guard);
        if storage_result.status != PbftManagerTransitionStorageStatus::Applied {
            return Ok(Self::rejected_lifecycle_transition(
                manager.state.snapshot(),
                storage_result.error_code,
            ));
        }

        manager.state.apply_committed_transition(&plan);
        manager
            .state
            .record_committed_reset(request.target_period, &plan);
        Ok(PbftManagerLifecycleTransitionOutcome {
            status: PbftManagerTransitionStorageStatus::Applied,
            snapshot: manager.state.snapshot(),
            remove_cert_voted_sidecar: plan.remove_cert_voted_block,
            clear_broadcasted_vote_sidecars: plan.clear_broadcasted_votes,
            reset_current_round_timer: plan.reset_current_round_start,
            reset_second_finish_timer: plan.reset_second_finish_start,
            print_cert_step_info: plan.print_cert_step_info,
            print_second_finish_step_info: plan.print_second_finish_step_info,
            reset_executed_block_follow_up: plan.reset_executed_block_status,
            error_code: String::new(),
        })
    }

    /// Returns a coherent snapshot of manager-owned period-data queue state.
    pub fn period_data_queue_snapshot(&self) -> Result<PeriodDataQueueSnapshot> {
        let manager = self.manager_state();
        let chain = manager.chain.head();
        let current_period = chain
            .size
            .checked_add(1)
            .context("PBFT_PERIOD_DATA_QUEUE_CHAIN_PERIOD_OVERFLOW")?;
        Ok(manager.period_data_queue.snapshot(
            chain.size,
            current_period,
            chain.last_pbft_block_hash,
        ))
    }

    /// Clears all manager-owned period-data queue metadata.
    pub fn clear_period_data_queue(&self) {
        self.manager_state().period_data_queue.clear();
    }

    /// Admits one complete period-data queue request under manager ownership.
    ///
    /// Sequencing rejection is represented in the returned outcome. Checked
    /// arithmetic errors are returned without partial mutation.
    pub fn push_period_data_queue(
        &self,
        request: PeriodDataQueuePushRequest,
    ) -> Result<PeriodDataQueuePushOutcome> {
        self.manager_state().period_data_queue.push(request)
    }

    /// Decodes and admits one encoded period-data payload.
    ///
    /// Deterministic payload inspection completes before the manager lock is
    /// acquired, so malformed bytes cannot partially mutate queue state.
    pub fn push_encoded_period_data_queue(
        &self,
        request: EncodedPeriodDataQueuePushRequest,
    ) -> Result<PeriodDataQueuePushOutcome> {
        self.manager.push_encoded_period_data_queue(request)
    }

    /// Pops the next manager-owned queue entry and certificate-source plan.
    ///
    /// An empty queue returns an error and leaves state unchanged.
    pub fn pop_period_data_queue(&self) -> Result<PeriodDataQueuePopPlan> {
        self.manager_state().period_data_queue.pop()
    }

    /// Removes queue entries older than `period`.
    ///
    /// Returns the count of entries removed from the live queue.
    pub fn clean_old_period_data_queue(&self, period: u64) -> usize {
        self.manager_state()
            .period_data_queue
            .clean_old_data(period)
    }

    fn run_finalization_executor_task(
        &self,
        task: impl FnOnce(&mut PbftManagerGuard<'_>) -> Result<PbftFinalizationOwnedActionDrain>,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        let mut manager = self.manager_state();
        let result = task(&mut manager);
        let drain = manager.finish_finalization_executor(result)?;
        Ok(PbftFinalizationExecutorBoundary {
            next_step: drain.next_step,
            cleared_anchor_dag_cache: drain.cleared_anchor_dag_cache,
            has_snapshot: drain.has_snapshot,
            expired_dag_hashes: drain.expired_dag_hashes,
            refresh_dag_counters: drain.refresh_dag_counters,
            snapshot: manager.state.snapshot(),
            error_code: drain.error_code,
        })
    }

    /// Plans one finalization dynamic-lambda update with native storage facts.
    ///
    /// The service derives the prior-period lookup from its manager-owned
    /// storage handle. Active accepted plans query the closest persisted lambda
    /// at or before `finalized_period - 1`. Period zero has no predecessor and
    /// returns an empty lookup without reading a period-zero row. Disabled or
    /// rejected plans likewise return an empty lookup. Storage failures are
    /// returned without mutating manager or durable state.
    pub fn plan_finalization_dynamic_lambda(
        &self,
        fact: PbftDynamicLambdaFact,
    ) -> Result<PbftFinalizationDynamicLambdaDecision> {
        let dynamic_lambda_active = fact.dynamic_lambda_active;
        let finalized_period = fact.finalized_period;
        let plan = plan_pbft_dynamic_lambda(fact);
        let last_saved_period_lambda = if dynamic_lambda_active
            && plan.status == PbftFinalizationStatus::Accepted
            && finalized_period > 0
        {
            let manager = self.manager_state();
            load_pbft_finalization_last_period_lambda(
                manager.storage.as_ref(),
                finalized_period.saturating_sub(1),
            )?
        } else {
            PbftFinalizationPeriodLambdaLookup {
                found: false,
                value: 0,
            }
        };
        Ok(PbftFinalizationDynamicLambdaDecision {
            plan,
            last_saved_period_lambda,
        })
    }

    /// Plans one PBFT finalization intent from live chain state.
    ///
    /// Inputs are PBFT candidate facts excluding live-chain-derived fields.
    /// The service samples current chain head state and derives `chain_last_hash`,
    /// `chain_last_period`, and legacy head payload bytes:
    /// - `chain_last_hash` is current chain `last_pbft_block_hash`
    /// - `chain_last_period` is chain size for non-duplicates, or
    ///   `block_period - 1` for already-in-chain candidates
    /// - `pbft_head_payload` is empty for already-in-chain candidates, otherwise
    ///   the legacy projected head payload.
    ///
    /// Lock-poisoned chain state returns `PBFT_CHAIN_SERVICE_LOCK_POISONED`;
    /// otherwise this method is pure and side-effect-free.
    pub fn plan_finalization_intent(
        &self,
        fact: PbftFinalizationIntent,
    ) -> Result<PbftFinalizationPlan> {
        let block_in_chain = self.chain().block_exists(fact.block_hash)?;
        let projected_anchor = !fact.pivot_dag_anchor_hash.is_zero();
        let (chain, pbft_head_payload) = self.chain().finalization_snapshot(
            fact.block_hash,
            (!block_in_chain).then_some(projected_anchor),
        )?;
        let chain_last_period = if block_in_chain {
            fact.block_period.saturating_sub(1)
        } else {
            chain.size
        };

        let domain_fact = PbftFinalizationIntentFact {
            block_hash: fact.block_hash,
            pbft_head_hash: if block_in_chain {
                fact.block_prev_hash
            } else {
                chain.head_hash
            },
            block_period: fact.block_period,
            block_prev_hash: fact.block_prev_hash,
            chain_last_hash: chain.last_pbft_block_hash,
            chain_last_period,
            block_in_chain,
            pivot_dag_anchor_hash: fact.pivot_dag_anchor_hash,
            has_pillar_block: fact.has_pillar_block,
            pillar_block_finalized: fact.pillar_block_finalized,
            request_dynamic_lambda_update: fact.request_dynamic_lambda_update,
            cert_vote_count: fact.cert_vote_count,
            sample_cert_vote_block_hash: fact.sample_cert_vote_block_hash,
            sample_cert_vote_period: fact.sample_cert_vote_period,
            sample_cert_vote_round: fact.sample_cert_vote_round,
            sample_cert_vote_step: fact.sample_cert_vote_step,
            block_lambda: fact.block_lambda,
            last_saved_period_lambda_found: fact.last_saved_period_lambda_found,
            last_saved_period_lambda: fact.last_saved_period_lambda,
            dynamic_blocks_per_year: fact.dynamic_blocks_per_year,
            rounds_count_dynamic_lambda: fact.rounds_count_dynamic_lambda,
            dynamic_lambda: fact.dynamic_lambda,
            dpos_blocks_per_year: fact.dpos_blocks_per_year,
            pbft_head_payload,
            period_data_rlp: fact.period_data_rlp,
            ordered_dag_block_hashes: fact.ordered_dag_block_hashes,
            ordered_transaction_hashes: fact.ordered_transaction_hashes,
            process_pillar_block_after_advance: fact.process_pillar_block_after_advance,
        };

        Ok(plan_pbft_finalization_intent(domain_fact))
    }

    /// Restores the coherent native PBFT service graph from shared storage.
    ///
    /// Errors preserve construction order: pillar schedule and slashing
    /// configuration are checked first, followed by chain, verified votes,
    /// proposed blocks, manager runtime, pillar restoration, and network
    /// composition. No service root escapes on failure.
    pub fn restore(storage: Arc<Storage>, config: PbftServiceConfig) -> Result<Self> {
        anyhow::ensure!(
            config.ficus_activation_period == u64::MAX || config.pillar_blocks_interval > 1,
            "PBFT_SERVICE_PILLAR_BLOCKS_INTERVAL_MUST_EXCEED_ONE"
        );
        let slashing = SlashingProofService::new(
            config.report_malicious_behaviour,
            config.magnolia_activation_period,
            SLASHING_PROOF_CACHE_MAX_SIZE,
            SLASHING_PROOF_CACHE_DELETE_STEP,
        )?;
        let chain = PbftChainService::restore(storage.clone())?;
        let verified_votes = PbftVerifiedVotesService::restore(storage.clone())?;
        let proposed_blocks = ProposedBlocksService::restore(storage.clone())?;
        let chain_head = chain.head();
        let runtime = create_pbft_manager_runtime_from_storage(
            &storage,
            PbftManagerStorageStartupFact {
                current_period: chain_head.size.saturating_add(1),
                cacti_active_at_chain_size: chain_head.size >= config.cacti_block,
                genesis_lambda_ms: config.genesis_lambda_ms,
                cacti_lambda_max_ms: config.cacti_lambda_max_ms,
                cacti_lambda_default_ms: config.cacti_lambda_default_ms,
                cacti_block: config.cacti_block,
                max_exponential_lambda_ms: config.max_exponential_lambda_ms,
                max_steps: config.max_steps,
                deadline_ms: config.deadline_ms,
                polling_interval_ms: config.polling_interval_ms,
            },
        )?;
        let pillar = PillarChainService::restore(storage.clone())?;
        let manager = PbftManagerService::new(runtime, storage.clone(), chain.clone());
        let network = ConsensusNetworkService::new(
            pillar.clone(),
            verified_votes.clone(),
            chain.clone(),
            proposed_blocks.clone(),
            manager.clone(),
            storage.clone(),
            config.ficus_activation_period,
            config.pillar_blocks_interval,
            config.sync_level_size,
            config.is_light_node,
            config.light_node_history,
        )?;

        Ok(Self {
            manager,
            chain,
            proposed_blocks,
            verified_votes,
            slashing,
            readiness: PbftServiceReadiness::pending(),
            pillar,
            network,
            sync_ingress: Mutex::new(None),
            sync_cert_bundle: Mutex::new(PbftSyncCertBundleRuntime {
                next_session_id: 1,
                active: None,
                last_report: None,
            }),
            committee_size: config.committee_size,
            number_of_proposers: config.number_of_proposers,
            slashing_enabled: config.report_malicious_behaviour,
        })
    }

    /// Returns an owned snapshot of the current native PBFT chain head.
    ///
    /// The snapshot is coherent under the chain sibling's read lock and is
    /// independent of later updates. A poisoned lock follows the chain
    /// service's fatal panic policy.
    pub fn pbft_chain_head(&self) -> PbftChainHead {
        self.chain().head()
    }

    /// Applies one in-memory PBFT head transition and returns the new snapshot.
    ///
    /// `block_hash` is the next finalized PBFT block and `anchor_hash` is its
    /// DAG anchor, where zero denotes a null anchor. The chain sibling owns
    /// checked size arithmetic and locking; overflow returns an error without
    /// partial mutation. Linkage validation is intentionally a separate task,
    /// and this operation performs no storage write because durable publication
    /// is owned by finalization.
    pub fn pbft_chain_update(&self, block_hash: H256, anchor_hash: H256) -> Result<PbftChainHead> {
        self.chain().update(block_hash, anchor_hash)
    }

    /// Checks native storage for one finalized PBFT block hash.
    ///
    /// The query uses the storage handle owned by the chain sibling and does
    /// not mutate its live head. Backend and index errors are propagated.
    pub fn pbft_chain_block_exists(&self, block_hash: H256) -> Result<bool> {
        self.chain().block_exists(block_hash)
    }

    /// Validates whether a candidate period and previous hash extend the live head.
    ///
    /// The chain sibling samples one coherent head under its read lock and
    /// returns a typed valid, period-mismatch, or previous-hash-mismatch result.
    /// Validation is side-effect-free; poisoned locks follow the sibling's
    /// fatal panic policy.
    pub fn pbft_chain_validate_block(&self, period: u64, prev_hash: H256) -> PbftBlockValidation {
        self.chain().validate_block(period, prev_hash)
    }

    /// Drives ordinary PBFT block validation through every native dependency.
    ///
    /// `candidate` supplies immutable candidate shape, a possible DAG executor
    /// report, and inputs that previously required C++ facade materialization.
    /// Internally owned statuses are discarded and recomputed so callers cannot
    /// bypass a native dependency. The task repeatedly
    /// drives the block-validation planner through PBFT-chain linkage, delayed
    /// FinalChain hash comparison, reward-vote selection, current pillar
    /// anchor validation, and inline DAG preparation. It returns terminal
    /// `Accept`, `Reject`, or `WaitForFinalization` plans.
    ///
    /// No manager guard is acquired or retained: each sibling or FinalChain
    /// call completes before the next stateless planner transition. FinalChain,
    /// verified-vote, pillar readiness, storage, and lock errors propagate;
    /// deterministic missing or mismatching facts remain typed planner results.
    pub fn validate_pbft_block_composed(
        &self,
        final_chain: &FinalChain,
        dag_transaction_service: &DagTransactionService,
        candidate: PbftBlockValidationCandidate,
    ) -> Result<crate::pbft_manager::PbftManagerBlockValidationPlan> {
        use crate::pbft_manager::PbftManagerBlockValidationFactStatus as FactStatus;

        let PbftBlockValidationCandidate {
            mut fact,
            previous_pbft_block_hash,
            candidate_final_chain_hash,
            expected_order_hash,
            pbft_gas_limit,
            reward_vote_hashes,
            pillar_block_hash,
        } = candidate;
        fact.pbft_chain_status = FactStatus::NotChecked;
        fact.final_chain_hash_status = FactStatus::NotChecked;
        fact.reward_votes_status = FactStatus::NotChecked;
        fact.dag_order_status = FactStatus::NotChecked;
        fact.dag_weight_status = FactStatus::NotChecked;
        fact.pivot_is_null = fact.pivot_hash == H256::zero();
        fact.dag_order_required = true;
        fact.pillar_block_status = if fact.pillar_block_required {
            FactStatus::NotChecked
        } else {
            FactStatus::NotRequired
        };
        let mut session = crate::pbft_manager::create_pbft_manager_block_validation_session(fact);
        let mut plan =
            crate::pbft_manager::next_pbft_manager_block_validation_session(&mut session);
        loop {
            use crate::pbft_manager::{
                PbftManagerBlockValidationAction as Action,
                PbftManagerBlockValidationNextCheck as NextCheck,
            };

            if plan.action != Action::RunCheck {
                return Ok(plan);
            }
            let status = match plan.next_check {
                NextCheck::CheckPbftChain => {
                    if matches!(
                        self.chain()
                            .try_validate_block(session.fact.period, previous_pbft_block_hash,)?,
                        PbftBlockValidation::Valid
                    ) {
                        FactStatus::Valid
                    } else {
                        FactStatus::Invalid
                    }
                }
                NextCheck::ValidateFinalChainHash => {
                    match final_chain.pbft_final_chain_hash(session.fact.period)? {
                        Some(expected) if H256(expected) == candidate_final_chain_hash => {
                            FactStatus::Valid
                        }
                        Some(_) => FactStatus::Invalid,
                        None => FactStatus::Missing,
                    }
                }
                NextCheck::CheckRewardVotes => {
                    if self
                        .select_reward_vote_payloads(
                            session.fact.period,
                            reward_vote_hashes.clone(),
                        )?
                        .accepted
                    {
                        FactStatus::Valid
                    } else {
                        FactStatus::Invalid
                    }
                }
                NextCheck::ValidatePillarBlock => {
                    let decision = self.plan_pillar_current_anchor_decision(
                        PillarCurrentAnchorDecisionRequest::ValidateCandidate {
                            candidate_hash: pillar_block_hash,
                        },
                    )?;
                    if decision.plan.selected {
                        FactStatus::Valid
                    } else {
                        match decision.plan.status {
                            crate::pillar_chain::PillarCurrentAnchorDecisionStatus::MissingCurrentAnchor
                            | crate::pillar_chain::PillarCurrentAnchorDecisionStatus::MissingCandidate => {
                                FactStatus::Missing
                            }
                            _ => FactStatus::Invalid,
                        }
                    }
                }
                NextCheck::CheckDagOrder => {
                    let (dag_order_status, dag_weight_status) = match self.prepare_candidate_dag(
                        dag_transaction_service,
                        session.fact.period,
                        session.fact.pivot_hash,
                        expected_order_hash,
                        pbft_gas_limit,
                    )? {
                        crate::pbft_manager::PbftCandidateDagPreparationStatus::Valid => {
                            (FactStatus::Valid, FactStatus::Valid)
                        }
                        crate::pbft_manager::PbftCandidateDagPreparationStatus::Missing => {
                            (FactStatus::Missing, FactStatus::NotRequired)
                        }
                        crate::pbft_manager::PbftCandidateDagPreparationStatus::OrderHashInvalid => {
                            (FactStatus::Invalid, FactStatus::NotRequired)
                        }
                        crate::pbft_manager::PbftCandidateDagPreparationStatus::WeightInvalid => {
                            (FactStatus::Valid, FactStatus::Invalid)
                        }
                    };
                    session.fact.dag_order_status = dag_order_status;
                    session.fact.dag_weight_status = dag_weight_status;
                    plan = crate::pbft_manager::next_pbft_manager_block_validation_session(
                        &mut session,
                    );
                    continue;
                }
                NextCheck::ValidateExtraData
                | NextCheck::CheckDagWeight
                | NextCheck::None
                | NextCheck::Unknown => return Ok(plan),
            };
            plan = crate::pbft_manager::report_pbft_manager_block_validation_session_check(
                &mut session,
                status,
            );
        }
    }

    /// Admits one proposed PBFT block through the complete native service graph.
    ///
    /// The operation resolves the native proposal entry, verifies its canonical
    /// period/hash/pivot identity, decodes all immutable validation inputs,
    /// drives composed PBFT/FinalChain/reward/pillar/DAG validation, and marks
    /// the entry valid only after an accepted terminal plan. Already-valid
    /// entries bypass repeated dependency work but still undergo canonical
    /// identity decoding. Missing and deterministic validation failures are
    /// typed outcomes; malformed RLP and dependency failures are errors.
    pub fn admit_proposed_block(
        &self,
        final_chain: &FinalChain,
        dag_transaction_service: &DagTransactionService,
        request: PbftProposedBlockAdmissionRequest,
    ) -> Result<PbftProposedBlockAdmissionResult> {
        let Some(entry) = self.proposed_block(request.period, request.block_hash) else {
            return Ok(PbftProposedBlockAdmissionResult {
                status: PbftProposedBlockAdmissionStatus::Missing,
                block_rlp: Vec::new(),
                error_code: "PBFT_PROPOSED_BLOCK_ADMISSION_MISSING",
            });
        };
        let candidate = proposed_block_validation_candidate(&entry, request)?;
        if entry.is_valid {
            return Ok(PbftProposedBlockAdmissionResult {
                status: PbftProposedBlockAdmissionStatus::AcceptedAlreadyValid,
                block_rlp: entry.block_rlp,
                error_code: "PBFT_PROPOSED_BLOCK_ADMISSION_ALREADY_VALID",
            });
        }

        let plan =
            self.validate_pbft_block_composed(final_chain, dag_transaction_service, candidate)?;
        match plan.action {
            crate::pbft_manager::PbftManagerBlockValidationAction::Accept => {
                self.mark_proposed_block_valid(request.period, request.block_hash)?;
                Ok(PbftProposedBlockAdmissionResult {
                    status: PbftProposedBlockAdmissionStatus::AcceptedNewlyValidated,
                    block_rlp: entry.block_rlp,
                    error_code: "PBFT_PROPOSED_BLOCK_ADMISSION_VALIDATED",
                })
            }
            crate::pbft_manager::PbftManagerBlockValidationAction::Reject
            | crate::pbft_manager::PbftManagerBlockValidationAction::WaitForFinalization => {
                Ok(PbftProposedBlockAdmissionResult {
                    status: PbftProposedBlockAdmissionStatus::Rejected,
                    block_rlp: Vec::new(),
                    error_code: plan.error_code,
                })
            }
            crate::pbft_manager::PbftManagerBlockValidationAction::ContractError => Err(anyhow!(
                "native proposed-block validation contract failed: {}",
                plan.error_code
            )),
            crate::pbft_manager::PbftManagerBlockValidationAction::RunCheck => Err(anyhow!(
                "native proposed-block validation returned non-terminal action: {}",
                plan.error_code
            )),
        }
    }

    /// Selects the leader among already-signed local proposal candidates.
    ///
    /// Every candidate is canonically decoded and identity-bound. Proposal
    /// votes are revalidated against native FinalChain/VRF facts, blocks are
    /// checked against the native PBFT chain and composed validation graph,
    /// and the existing native planner owns status derivation and ranking.
    /// The task neither publishes a proposal nor mutates the proposal cache;
    /// it returns only the original input index for the retained C++ signing
    /// and publication boundary.
    pub fn select_local_proposal_candidate(
        &self,
        final_chain: &FinalChain,
        dag_transaction_service: &DagTransactionService,
        request: PbftLocalProposalSelectionRequest,
    ) -> Result<PbftLocalProposalSelectionResult> {
        use crate::pbft_manager::{
            PbftManagerBlockValidationAction, PbftManagerLeaderBlockValidationStatus,
            PbftManagerLeaderCandidateInputFact, plan_pbft_manager_leader_candidates,
        };

        let mut identities = Vec::with_capacity(request.candidates.len());
        let mut facts = Vec::with_capacity(request.candidates.len());
        for candidate in request.candidates {
            let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&candidate.block_rlp))?;
            let entry = ProposedBlockEntry {
                period: link.period,
                block_hash: link.block_hash,
                block_rlp: candidate.block_rlp,
                pivot_hash: link.pivot_dag_block_hash,
                is_valid: false,
            };
            let block_candidate = proposed_block_validation_candidate(
                &entry,
                PbftProposedBlockAdmissionRequest {
                    period: request.period,
                    block_hash: link.block_hash,
                    pbft_gas_limit: request.pbft_gas_limit,
                    extra_data_required: request.extra_data_required,
                    pillar_block_required: request.pillar_block_required,
                },
            )?;
            let inspection = inspect_canonical_pbft_vote(&candidate.vote_rlp)?;
            ensure!(
                inspection.status == PbftCanonicalVoteInspectionStatus::Valid
                    && inspection.signature_valid,
                "PBFT_LOCAL_PROPOSAL_INVALID_VOTE_SIGNATURE"
            );
            ensure!(
                inspection.period == request.period
                    && inspection.round == request.round
                    && inspection.vote_type == PbftVoteType::Propose,
                "PBFT_LOCAL_PROPOSAL_VOTE_CONTEXT_MISMATCH"
            );
            ensure!(
                inspection.block_hash == link.block_hash,
                "PBFT_LOCAL_PROPOSAL_BLOCK_VOTE_MISMATCH"
            );

            let (validation, _) = self
                .validate_verified_vote_with_final_chain_internal(
                    final_chain,
                    &candidate.vote_rlp,
                    PbftVoteAdmissionValidationRequest {
                        strict_vrf: true,
                        committee_size: self.committee_size,
                        number_of_proposers: self.number_of_proposers,
                        has_preverified_weight: false,
                        preverified_weight: 0,
                    },
                    false,
                )
                .context("PBFT_LOCAL_PROPOSAL_VOTE_VALIDATION")?;
            let valid_weight = validation.accepted
                && validation.weight_calculated
                && validation.calculated_weight > 0;
            if valid_weight {
                ensure!(
                    inspection.has_embedded_weight
                        && inspection.embedded_weight == validation.calculated_weight,
                    "PBFT_LOCAL_PROPOSAL_EMBEDDED_WEIGHT_MISMATCH"
                );
            }
            let block_in_chain = self
                .pbft_chain_block_exists(link.block_hash)
                .context("PBFT_LOCAL_PROPOSAL_CHAIN_LOOKUP")?;
            let block_validation_status = if !valid_weight || block_in_chain {
                PbftManagerLeaderBlockValidationStatus::Rejected
            } else {
                let validation_plan = self
                    .validate_pbft_block_composed(
                        final_chain,
                        dag_transaction_service,
                        block_candidate,
                    )
                    .context("PBFT_LOCAL_PROPOSAL_BLOCK_VALIDATION")?;
                match validation_plan.action {
                    PbftManagerBlockValidationAction::Accept => {
                        PbftManagerLeaderBlockValidationStatus::Validated
                    }
                    PbftManagerBlockValidationAction::Reject
                    | PbftManagerBlockValidationAction::WaitForFinalization => {
                        PbftManagerLeaderBlockValidationStatus::Rejected
                    }
                    PbftManagerBlockValidationAction::ContractError
                    | PbftManagerBlockValidationAction::RunCheck => {
                        return Err(anyhow!(
                            "native local proposal block validation returned non-terminal action: {}",
                            validation_plan.error_code
                        ));
                    }
                }
            };

            identities.push((inspection.vote_hash, link.block_hash));
            facts.push(PbftManagerLeaderCandidateInputFact {
                vote_hash: inspection.vote_hash,
                block_hash: link.block_hash,
                period: request.period,
                credential: validation.vrf_output,
                voter_public_key: inspection.recovered_public_key,
                weight_found: valid_weight,
                weight: validation.calculated_weight,
                block_in_chain,
                proposed_block_found: true,
                block_validation_status,
                pivot_hash: link.pivot_dag_block_hash,
            });
        }

        let plan = plan_pbft_manager_leader_candidates(facts);
        if !plan.selected {
            return Ok(PbftLocalProposalSelectionResult {
                selected: false,
                selected_index: 0,
                error_code: plan.error_code,
            });
        }
        let selected_index = identities
            .iter()
            .rposition(|(vote_hash, block_hash)| {
                *vote_hash == plan.selected_vote_hash && *block_hash == plan.selected_block_hash
            })
            .ok_or_else(|| anyhow!("PBFT_LOCAL_PROPOSAL_SELECTED_UNKNOWN_CANDIDATE"))?;
        Ok(PbftLocalProposalSelectionResult {
            selected: true,
            selected_index: u64::try_from(selected_index)
                .context("PBFT_LOCAL_PROPOSAL_SELECTED_INDEX_OVERFLOW")?,
            error_code: plan.error_code,
        })
    }

    /// Persists and publishes one proposed PBFT block through native service state.
    ///
    /// The supplied identity must match the canonical signed block RLP. The
    /// proposal sibling holds one write lock across validation, an unconditional
    /// durable overwrite, duplicate detection, and possible live insertion.
    /// Storage commits before memory publication. The result is `true` only for
    /// a new live entry; a duplicate returns `false` after repairing its durable
    /// row. Validation, storage, and lock failures are returned without making
    /// live state lead durable state.
    pub fn publish_proposed_block(
        &self,
        period: u64,
        block_hash: H256,
        pivot_hash: H256,
        block_rlp: Vec<u8>,
    ) -> Result<bool> {
        self.proposed_blocks()
            .push_with_storage(period, block_hash, pivot_hash, block_rlp)
    }

    /// Publishes one canonical signed proposed block from its native bytes.
    ///
    /// Rust decodes period, canonical signed-block hash, and pivot DAG hash
    /// before entering the existing durable-first proposal publication path.
    /// Malformed bytes return an error without storage or live-state mutation.
    /// `true` means a new live proposal was published; `false` means the
    /// durable row was repaired for an already-present proposal.
    pub fn publish_proposed_block_effect(
        &self,
        canonical_signed_block_rlp: Vec<u8>,
    ) -> Result<bool> {
        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(
            canonical_signed_block_rlp.as_slice(),
        ))?;
        self.publish_proposed_block(
            link.period,
            link.block_hash,
            link.pivot_dag_block_hash,
            canonical_signed_block_rlp,
        )
    }

    /// Marks one proposed PBFT block as valid in process-local state.
    ///
    /// The period and hash must identify an existing entry. Missing entries and
    /// lock failure return errors without mutation. This task performs no block
    /// validation and no storage write; callers invoke it only after the
    /// retained external validator succeeds.
    pub fn mark_proposed_block_valid(&self, period: u64, block_hash: H256) -> Result<()> {
        self.proposed_blocks().mark_valid(period, block_hash)
    }

    /// Returns an owned proposed-block entry from process-local state.
    ///
    /// The lookup is side-effect-free and returns `None` for a missing period
    /// and hash. Canonical RLP and the process-local validation bit are cloned
    /// while the sibling read lock is held; lock poison follows its fatal panic
    /// policy.
    pub fn proposed_block(&self, period: u64, block_hash: H256) -> Option<ProposedBlockEntry> {
        self.proposed_blocks().get(period, block_hash)
    }

    /// Publishes completion of PBFT startup replay.
    pub fn complete_bootstrap(&self) {
        self.readiness.mark_ready();
    }

    /// Atomically cleans service-owned period state after PBFT finalization.
    ///
    /// `finalized_chain_size` must be nonzero and `new_period` must be its exact
    /// checked successor. The operation acquires verified votes before proposed
    /// blocks, plans both cleanups, commits all durable proposed-block deletes
    /// in one Rust storage batch, and only then publishes exact in-memory
    /// removals. Valid no-op transitions are published with zero counts.
    /// Validation or storage failures return a typed rejected result without
    /// memory publication; lock poison remains an operational error.
    #[cfg(test)]
    pub(crate) fn cleanup_period_state(
        &self,
        finalized_chain_size: u64,
        new_period: u64,
    ) -> Result<PbftPeriodStateCleanupResult> {
        cleanup_period_state_with_commit(
            self.verified_votes(),
            self.proposed_blocks(),
            finalized_chain_size,
            new_period,
            |storage, batch| {
                storage
                    .commit_write_batch_with_sync(batch, false)
                    .context("PBFT_PERIOD_STATE_CLEANUP_COMMIT")
            },
        )
    }

    /// Commits one externally executed PBFT period advance under native ownership.
    ///
    /// The manager reset provenance is validated before cleanup. Rust then
    /// acquires verified-vote and proposed-block siblings in the canonical
    /// manager-first order, commits durable cleanup before live cleanup
    /// publication, and publishes the new manager period only after cleanup
    /// succeeds. Invalid or duplicate period reports return the unchanged
    /// rejected manager snapshot; storage and cleanup failures return an error
    /// while preserving reset provenance so the operation can be retried.
    pub fn apply_period_advance(
        &self,
        new_period: u64,
    ) -> Result<crate::pbft_manager::PbftManagerRuntimeSnapshot> {
        self.apply_period_advance_with_commit(new_period, |storage, batch| {
            storage
                .commit_write_batch_with_sync(batch, false)
                .context("PBFT_PERIOD_ADVANCE_CLEANUP_COMMIT")
        })
    }

    /// Applies one period advance with an injected durable cleanup commit.
    ///
    /// This is the single native implementation behind the production commit
    /// boundary. The injected operation is used by tests to prove that a
    /// durable-write failure leaves manager reset provenance and both cleanup
    /// siblings unchanged, allowing the same transition to be retried.
    pub(crate) fn apply_period_advance_with_commit<F>(
        &self,
        new_period: u64,
        commit: F,
    ) -> Result<crate::pbft_manager::PbftManagerRuntimeSnapshot>
    where
        F: FnOnce(&Storage, StorageWriteBatch) -> Result<()>,
    {
        let Some(finalized_chain_size) = new_period.checked_sub(1) else {
            return Ok(self
                .manager
                .lock()
                .state
                .apply_committed_period_advance(new_period));
        };
        let mut manager = self.manager.lock();
        let plan = manager
            .state
            .plan_advance_period_after_reset(finalized_chain_size);
        if !plan.accepted || plan.new_period != new_period {
            return Ok(manager.state.apply_committed_period_advance(new_period));
        }

        let mut snapshot = None;
        let cleanup = cleanup_period_state_with_commit_and_publish(
            self.verified_votes(),
            self.proposed_blocks(),
            finalized_chain_size,
            new_period,
            commit,
            || {
                snapshot = Some(manager.state.apply_committed_period_advance(new_period));
            },
        )?;
        if cleanup.status == PbftPeriodStateCleanupStatus::Rejected || !cleanup.transition_published
        {
            return Err(anyhow::Error::msg(cleanup.error_code));
        }

        snapshot.context("PBFT_PERIOD_ADVANCE_PUBLICATION_MISSING")
    }

    /// Starts or resumes one PBFT finalization executor under native ownership.
    ///
    /// The application root acquires the manager serialization domain, invokes
    /// the complete lock-held start/resume task against the supplied native
    /// DAG/transaction sibling, and captures the compatibility snapshot before
    /// releasing the manager lock. The first external action is returned as a
    /// typed boundary; no C++ callback occurs while native locks are held.
    ///
    /// Operational errors and terminal outcomes clear retained executor state
    /// inside the manager task. Active boundaries retain their plan, cursor,
    /// authenticated reward-reset generation, and prepared sortition request.
    pub fn start_finalization_executor(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        request: PbftFinalizationExecutorStartRequest,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        let mut manager = self.manager_state();
        let drain = manager.start_finalization_executor(
            dag_transaction_service,
            self.verified_votes(),
            request,
        )?;
        Ok(PbftFinalizationExecutorBoundary {
            next_step: drain.next_step,
            cleared_anchor_dag_cache: drain.cleared_anchor_dag_cache,
            has_snapshot: drain.has_snapshot,
            expired_dag_hashes: drain.expired_dag_hashes,
            refresh_dag_counters: drain.refresh_dag_counters,
            snapshot: manager.state.snapshot(),
            error_code: drain.error_code,
        })
    }

    /// Reports failure of the current external finalization leaf.
    ///
    /// The manager validates the echoed cursor, records the supplied external
    /// status and error, clears the terminal session, and captures the coherent
    /// compatibility snapshot before releasing its lock.
    pub fn fail_finalization_external_effect(
        &self,
        cursor: u32,
        status: u8,
        error_code: String,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let step = manager.fail_finalization_external_effect(cursor, status, error_code)?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Advances the finalized DAG-order external leaf.
    ///
    /// Rust derives the expected action and accepted write intent, performs
    /// native finalized-order mutation, drains subsequent manager-owned actions,
    /// and returns the next external boundary under one manager lock.
    pub fn advance_finalization_dag_order(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        cursor: u32,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let (step, expired_dag_hashes, refresh_dag_counters) =
                manager.advance_finalization_set_dag_order(dag_transaction_service, cursor)?;
            let mut drain = manager.continue_finalization_executor_from_step(step)?;
            drain.expired_dag_hashes = expired_dag_hashes;
            drain.refresh_dag_counters = refresh_dag_counters;
            Ok(drain)
        })
    }

    /// Advances a specific external finalization action reported by the boundary.
    ///
    /// The action is decoded from the canonical action code and mapped to the
    /// corresponding Rust-owned external-effect leaf implementation. Leaf-specific
    /// payloads are consumed only for matching actions.
    pub fn advance_finalization_action(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        cursor: u32,
        action: u8,
        last_block: u64,
        request_period: u64,
        retention_window: u64,
        account_nonce_facts: Vec<crate::transaction_service::TransactionServiceAccountNonceFact>,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        let finalization_action = PbftFinalizationRuntimeAction::from_u8(action)
            .ok_or_else(|| anyhow::anyhow!("PBFT_FINALIZE_UNKNOWN_ACTION"))?;

        match finalization_action {
            PbftFinalizationRuntimeAction::CommitSortitionRuntime => {
                self.advance_finalization_sortition_commit(dag_transaction_service, cursor)
            }
            PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime => {
                self.advance_finalization_reward_votes_reset(cursor)
            }
            PbftFinalizationRuntimeAction::SetDagBlockOrder => {
                self.advance_finalization_dag_order(dag_transaction_service, cursor)
            }
            PbftFinalizationRuntimeAction::UpdateFinalizedTransactions => self
                .advance_finalization_transaction_status(
                    dag_transaction_service,
                    cursor,
                    retention_window,
                    account_nonce_facts,
                ),
            PbftFinalizationRuntimeAction::FinalizeFinalChain => {
                self.advance_finalization_final_chain_dispatch(cursor, last_block)
            }
            PbftFinalizationRuntimeAction::AdvancePeriod => {
                self.advance_finalization_advance_period(cursor)
            }
            PbftFinalizationRuntimeAction::ProcessPillarBlock => {
                self.advance_finalization_pillar_post_processing(cursor, request_period)
            }
            _ => Err(anyhow::anyhow!("PBFT_FINALIZE_UNSUPPORTED_ACTION")),
        }
    }

    /// Commits native sortition state and advances finalization.
    ///
    /// Manager-before-sortition lock order is retained. The prepared request is
    /// consumed exactly once, validated against live sortition facts, followed
    /// by native owned-action draining and terminal cleanup.
    pub fn advance_finalization_sortition_commit(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        cursor: u32,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let step =
                manager.advance_finalization_sortition_commit(dag_transaction_service, cursor)?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Commits the native reward-vote cursor and advances finalization.
    ///
    /// The PBFT root composes its manager and verified-vote siblings in fixed
    /// order, validates reset provenance, drains manager-owned actions, and
    /// returns only the next external boundary.
    pub fn advance_finalization_reward_votes_reset(
        &self,
        cursor: u32,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let step =
                manager.advance_finalization_reward_votes_reset(self.verified_votes(), cursor)?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Applies finalized transaction status and advances finalization.
    ///
    /// The PBFT root composes manager-before-DAG/transaction ownership while
    /// C++ supplies only the retained external-EVM account nonce facts.
    pub fn advance_finalization_transaction_status(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        cursor: u32,
        retention_window: u64,
        account_nonce_facts: Vec<crate::transaction_service::TransactionServiceAccountNonceFact>,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let step = manager.advance_finalization_transaction_status(
                dag_transaction_service,
                cursor,
                retention_window,
                account_nonce_facts,
            )?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Reports successful FinalChain/EVM dispatch and advances finalization.
    ///
    /// Only the observed FinalChain height crosses this boundary. Rust derives
    /// blocks-per-year and every manager-owned identity from the retained plan.
    pub fn advance_finalization_final_chain_dispatch(
        &self,
        cursor: u32,
        last_block: u64,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let step =
                manager.advance_finalization_live_mutation(cursor, |action, write_set| {
                    let mut report = base_owned_finalization_live_report(action, write_set);
                    report.final_chain_dispatched = true;
                    report.final_chain_blocks_per_year = write_set.blocks_per_year;
                    report.final_chain_last_block = last_block;
                    report
                })?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Reports pillar post-processing facts and advances finalization.
    ///
    /// The manager period is sampled under the same serialization lock as
    /// cursor validation. Rust derives the processed period from its retained
    /// plan; callers supply only the request period observed at the pillar leaf.
    pub fn advance_finalization_pillar_post_processing(
        &self,
        cursor: u32,
        request_period: u64,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let manager_period = manager.state.snapshot().period;
            let step =
                manager.advance_finalization_live_mutation(cursor, |action, write_set| {
                    let mut report = base_owned_finalization_live_report(action, write_set);
                    report.manager_period = manager_period;
                    report.pillar_processed_period = write_set.block_period;
                    report.pillar_request_period = request_period;
                    report
                })?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Reports the native period-cleanup result and advances finalization.
    ///
    /// The resulting manager period is read from the lock-held native snapshot;
    /// action identity, cursor validation, owned draining, cleanup, and
    /// boundary capture remain native.
    pub fn advance_finalization_advance_period(
        &self,
        cursor: u32,
    ) -> Result<PbftFinalizationExecutorBoundary> {
        self.run_finalization_executor_task(|manager| {
            let manager_period = manager.state.snapshot().period;
            let step =
                manager.advance_finalization_live_mutation(cursor, |action, write_set| {
                    let mut report = base_owned_finalization_live_report(action, write_set);
                    report.manager_period = manager_period;
                    report
                })?;
            manager.continue_finalization_executor_from_step(step)
        })
    }

    /// Starts a manager-owned synced-period admission cursor when bootstrap is ready.
    ///
    /// The immutable candidate facts move directly into the native manager
    /// owner. A pending bootstrap rejects the command without allocating or
    /// replacing a session.
    pub fn begin_pbft_sync_admission(
        &self,
        fact: crate::pbft_sync::PbftSyncAdmissionInitialFact,
    ) -> bool {
        if !self.is_ready() {
            return false;
        }
        self.manager.begin_pbft_sync_admission(fact);
        true
    }

    /// Returns the current native synced-period admission step.
    ///
    /// `None` denotes either an incomplete bootstrap or no active cursor.
    /// Terminal/error steps are returned once and consumed by the manager.
    pub fn pbft_sync_admission_next(
        &self,
    ) -> Option<crate::pbft_sync::PbftSyncAdmissionSessionStep> {
        self.is_ready()
            .then(|| self.manager.pbft_sync_admission_next())
            .flatten()
    }

    /// Reports one non-transaction validation fact to the native admission cursor.
    ///
    /// Unknown or stale reports are converted by the native session into a
    /// terminal contract error and consume the cursor.
    pub fn report_pbft_sync_admission_status(
        &self,
        cursor: u32,
        check: crate::pbft_sync::PbftSyncProcessRuntimeNextCheck,
        final_chain_status: crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus,
        fact_status: crate::pbft_sync::PbftSyncFactStatus,
    ) -> Option<crate::pbft_sync::PbftSyncAdmissionSessionStep> {
        self.manager.report_pbft_sync_admission_status(
            cursor,
            check,
            final_chain_status,
            fact_status,
        )
    }

    /// Validates the exact active PBFT sync FinalChain hash and continues rewards.
    ///
    /// Generation, cursor, block period, and candidate hash are captured under
    /// the manager lock entirely from immutable session facts. The delayed
    /// FinalChain hash lookup runs after releasing that lock. Exact reporting
    /// atomically captures a resulting reward request, whose selection also
    /// runs unlocked and exact-reports through the existing continuation.
    /// Period-zero, missing, matching, and mismatching hashes preserve native
    /// FinalChain semantics. Infrastructure failure exact-aborts only the
    /// captured request; stale success or failure returns `None` without
    /// consuming a replacement session or exposing reward records.
    pub fn validate_pbft_sync_admission_final_chain_hash(
        &self,
        final_chain: &FinalChain,
    ) -> Option<(
        crate::pbft_sync::PbftSyncAdmissionSessionStep,
        Vec<crate::pbft_vote_payload::PbftVotePayloadRecord>,
        crate::pbft_manager::PbftManagerFinalChainHashValidationResult,
    )> {
        self.validate_pbft_sync_admission_final_chain_hash_with(final_chain, || {})
    }

    fn validate_pbft_sync_admission_final_chain_hash_with(
        &self,
        final_chain: &FinalChain,
        after_capture: impl FnOnce(),
    ) -> Option<(
        crate::pbft_sync::PbftSyncAdmissionSessionStep,
        Vec<crate::pbft_vote_payload::PbftVotePayloadRecord>,
        crate::pbft_manager::PbftManagerFinalChainHashValidationResult,
    )> {
        if !self.is_ready() {
            return None;
        }
        let identity = self
            .manager
            .pbft_sync_admission_final_chain_hash_request()?;
        after_capture();
        let expected = match final_chain.pbft_final_chain_hash(identity.block_period) {
            Ok(expected) => expected,
            Err(error) => {
                let step = self
                    .manager
                    .abort_pbft_sync_admission_final_chain_hash_exact(identity)?;
                return Some((
                    step,
                    Vec::new(),
                    crate::pbft_manager::PbftManagerFinalChainHashValidationResult {
                        status: crate::pbft_manager::PbftManagerFinalChainHashStatus::Unknown,
                        expected_hash: H256::zero(),
                        error_code: error.to_string(),
                    },
                ));
            }
        };
        let validation = match expected {
            Some(expected) if H256(expected) == identity.candidate_final_chain_hash => {
                crate::pbft_manager::PbftManagerFinalChainHashValidationResult {
                    status: crate::pbft_manager::PbftManagerFinalChainHashStatus::Valid,
                    expected_hash: H256(expected),
                    error_code: String::new(),
                }
            }
            Some(expected) => crate::pbft_manager::PbftManagerFinalChainHashValidationResult {
                status: crate::pbft_manager::PbftManagerFinalChainHashStatus::Invalid,
                expected_hash: H256(expected),
                error_code: "PBFT_MANAGER_FINAL_CHAIN_HASH_MISMATCH".to_string(),
            },
            None => crate::pbft_manager::PbftManagerFinalChainHashValidationResult {
                status: crate::pbft_manager::PbftManagerFinalChainHashStatus::Missing,
                expected_hash: H256::zero(),
                error_code: "PBFT_MANAGER_FINAL_CHAIN_HASH_MISSING".to_string(),
            },
        };
        let runtime_status = match validation.status {
            crate::pbft_manager::PbftManagerFinalChainHashStatus::Valid => {
                crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus::Valid
            }
            crate::pbft_manager::PbftManagerFinalChainHashStatus::Missing => {
                crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus::Missing
            }
            crate::pbft_manager::PbftManagerFinalChainHashStatus::Invalid => {
                crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus::Invalid
            }
            crate::pbft_manager::PbftManagerFinalChainHashStatus::Unknown => {
                crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus::Unknown
            }
        };
        let (step, reward_identity) = self
            .manager
            .report_pbft_sync_admission_final_chain_hash_exact(identity, runtime_status)?;
        let (step, records) = match reward_identity {
            Some(identity) => self.select_and_report_pbft_sync_reward_votes(identity)?,
            None => (step, Vec::new()),
        };
        Some((step, records, validation))
    }

    /// Reports one external status and performs any resulting reward selection.
    ///
    /// The predecessor report and reward-request capture share one manager
    /// critical section. If the transition requests reward votes, verified-vote
    /// selection runs after releasing the manager lock and exact-reports using
    /// the captured generation, cursor, period, and ordered hashes. Accepted
    /// records are returned in request order. Deterministic rejection exposes
    /// no records; infrastructure failure exact-aborts; stale success or
    /// failure returns `None` without consuming a replacement session.
    pub fn report_pbft_sync_admission_status_with_reward_votes(
        &self,
        cursor: u32,
        check: crate::pbft_sync::PbftSyncProcessRuntimeNextCheck,
        final_chain_status: crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus,
        fact_status: crate::pbft_sync::PbftSyncFactStatus,
    ) -> Option<(
        crate::pbft_sync::PbftSyncAdmissionSessionStep,
        Vec<crate::pbft_vote_payload::PbftVotePayloadRecord>,
    )> {
        self.report_pbft_sync_admission_status_with_reward_votes_with(
            cursor,
            check,
            final_chain_status,
            fact_status,
            || {},
        )
    }

    fn report_pbft_sync_admission_status_with_reward_votes_with(
        &self,
        cursor: u32,
        check: crate::pbft_sync::PbftSyncProcessRuntimeNextCheck,
        final_chain_status: crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus,
        fact_status: crate::pbft_sync::PbftSyncFactStatus,
        after_capture: impl FnOnce(),
    ) -> Option<(
        crate::pbft_sync::PbftSyncAdmissionSessionStep,
        Vec<crate::pbft_vote_payload::PbftVotePayloadRecord>,
    )> {
        let (step, identity) = self
            .manager
            .report_pbft_sync_admission_status_and_capture_reward_request(
                cursor,
                check,
                final_chain_status,
                fact_status,
            )?;
        let Some(identity) = identity else {
            return Some((step, Vec::new()));
        };
        after_capture();
        self.select_and_report_pbft_sync_reward_votes(identity)
    }

    /// Selects reward-vote payloads for the exact active PBFT sync request.
    ///
    /// Generation, cursor, block period, and ordered reward hashes are captured
    /// under the manager lock entirely from immutable session-start facts.
    /// Native verified-vote selection runs after releasing that lock. Accepted
    /// records retain request order and are returned beside the advanced step;
    /// deterministic rejection returns an empty record list and reports the
    /// invalid fact. Infrastructure failure exact-aborts the captured session.
    /// Stale success or failure returns `None` without mutating or exposing
    /// records to a replacement session.
    pub fn validate_pbft_sync_admission_reward_votes(
        &self,
    ) -> Option<(
        crate::pbft_sync::PbftSyncAdmissionSessionStep,
        Vec<crate::pbft_vote_payload::PbftVotePayloadRecord>,
    )> {
        self.validate_pbft_sync_admission_reward_votes_with(|| {})
    }

    fn validate_pbft_sync_admission_reward_votes_with(
        &self,
        after_capture: impl FnOnce(),
    ) -> Option<(
        crate::pbft_sync::PbftSyncAdmissionSessionStep,
        Vec<crate::pbft_vote_payload::PbftVotePayloadRecord>,
    )> {
        if !self.is_ready() {
            return None;
        }
        let identity = self.manager.pbft_sync_admission_reward_request()?;
        after_capture();
        self.select_and_report_pbft_sync_reward_votes(identity)
    }

    fn select_and_report_pbft_sync_reward_votes(
        &self,
        identity: crate::pbft_manager::PbftSyncAdmissionRewardRequestIdentity,
    ) -> Option<(
        crate::pbft_sync::PbftSyncAdmissionSessionStep,
        Vec<crate::pbft_vote_payload::PbftVotePayloadRecord>,
    )> {
        let selection = self.select_reward_vote_payloads(
            identity.block_period,
            identity.reward_vote_hashes.clone(),
        );
        match selection {
            Ok(selection) => {
                let accepted = selection.accepted;
                let step = self.manager.report_pbft_sync_admission_reward_votes_exact(
                    identity,
                    if accepted {
                        crate::pbft_sync::PbftSyncFactStatus::Valid
                    } else {
                        crate::pbft_sync::PbftSyncFactStatus::Invalid
                    },
                )?;
                Some((
                    step,
                    if accepted {
                        selection.selected_records
                    } else {
                        Vec::new()
                    },
                ))
            }
            Err(_) => self
                .manager
                .abort_pbft_sync_admission_reward_votes_exact(identity)
                .map(|step| (step, Vec::new())),
        }
    }

    /// Validates and applies the exact pillar-vote bundle requested by PBFT sync.
    ///
    /// The pending generation, cursor, and required PBFT block period are
    /// captured under the manager lock. Pillar inspection, FinalChain weight
    /// queries, and generation-bound pillar apply then run without that lock.
    /// Empty input, period zero, unavailable pillar state, deterministic bundle
    /// rejection, insertion failure, and every infrastructure error map to the
    /// legacy invalid fact. The result is reported only if the same admission
    /// identity is still pending; a stale replacement returns `None` without
    /// mutating the new cursor.
    pub fn validate_pbft_sync_admission_pillar_votes(
        &self,
        final_chain: &FinalChain,
        vote_rlps: Vec<PillarVoteRlpPayload>,
    ) -> Option<crate::pbft_sync::PbftSyncAdmissionSessionStep> {
        if !self.is_ready() {
            return None;
        }
        let identity = self.manager.pbft_sync_admission_pillar_request()?;
        let valid = identity.required_votes_period != 0
            && !vote_rlps.is_empty()
            && self
                .apply_pillar_vote_bundle_with_final_chain(
                    final_chain,
                    vote_rlps,
                    identity.required_votes_period,
                )
                .is_ok_and(|plan| plan.status == 0 && !plan.insert_failed);
        self.manager.report_pbft_sync_admission_pillar_status_exact(
            identity,
            if valid {
                crate::pbft_sync::PbftSyncFactStatus::Valid
            } else {
                crate::pbft_sync::PbftSyncFactStatus::Invalid
            },
        )
    }

    /// Performs the transaction work requested by the active PBFT sync admission.
    ///
    /// The exact manager generation, cursor, and ordered finalized-lookup
    /// hashes are captured before releasing the manager lock. Native
    /// transaction filtering and supplied-transaction verification then run
    /// against the composed DAG/transaction service; latest FinalChain sender
    /// nonces are sampled for verification, with absent accounts contributing
    /// nonce zero. Transaction, storage, FinalChain, and nonce-width failures
    /// become a terminal abort only for the exact captured request. A result is
    /// applied only when the same request remains pending; stale success or
    /// failure returns `None` and cannot mutate a replacement session or queue.
    pub fn validate_pbft_sync_admission_transactions(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        final_chain: &FinalChain,
        transactions: Vec<PeriodDataQueueTransactionIdentity>,
    ) -> Option<crate::pbft_sync::PbftSyncAdmissionSessionStep> {
        self.validate_pbft_sync_admission_transactions_with(
            dag_transaction_service,
            final_chain,
            transactions,
            || {},
        )
    }

    fn validate_pbft_sync_admission_transactions_with(
        &self,
        dag_transaction_service: &crate::dag_transaction_service::DagTransactionService,
        final_chain: &FinalChain,
        transactions: Vec<PeriodDataQueueTransactionIdentity>,
        after_capture: impl FnOnce(),
    ) -> Option<crate::pbft_sync::PbftSyncAdmissionSessionStep> {
        if !self.is_ready() {
            return None;
        }
        let identity = self.manager.pbft_sync_admission_transaction_request()?;
        after_capture();

        let report = (|| -> Result<crate::pbft_sync::PbftSyncAdmissionTransactionReport> {
            let missing_transaction_hashes = dag_transaction_service
                .transaction_filter_non_finalized(
                    identity
                        .finalized_lookup_hashes
                        .iter()
                        .copied()
                        .enumerate()
                        .map(|(index, hash)| {
                            Ok(crate::transaction_service::TransactionServiceFinalizedFilterRequest {
                                input_index: u64::try_from(index)
                                    .context("PBFT_SYNC_TRANSACTION_LOOKUP_INDEX_OVERFLOW")?,
                                hash,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?,
                )?
                .not_finalized
                .into_iter()
                .map(|action| action.hash)
                .collect();

            let mut verification_facts = Vec::with_capacity(transactions.len());
            for transaction in transactions {
                let sender_account_nonce = match final_chain.account(transaction.sender)? {
                    Some(account) => final_chain_nonce_as_u256(&account.nonce)?,
                    None => U256::zero(),
                };
                verification_facts.push(
                    crate::transaction_service::TransactionServiceVerifyNotFinalizedFact {
                        input_index: transaction.input_index,
                        hash: transaction.hash,
                        transaction_nonce: U256::from_big_endian(&transaction.transaction_nonce),
                        sender_account_nonce,
                    },
                );
            }
            let finalized =
                dag_transaction_service.transaction_verify_not_finalized(verification_facts)?;
            Ok(crate::pbft_sync::PbftSyncAdmissionTransactionReport {
                missing_transaction_hashes,
                finalized_transaction_hashes: finalized
                    .is_finalized
                    .then_some(finalized.hash)
                    .into_iter()
                    .collect(),
                contains_finalized_transactions: finalized.is_finalized,
            })
        })();
        match report {
            Ok(report) => self
                .manager
                .report_pbft_sync_admission_transactions_exact(identity, report),
            Err(_) => self
                .manager
                .abort_pbft_sync_admission_transactions_exact(identity),
        }
    }

    /// Aborts and consumes the current synced-period admission cursor.
    pub fn abort_pbft_sync_admission(
        &self,
    ) -> Option<crate::pbft_sync::PbftSyncAdmissionSessionStep> {
        self.manager.abort_pbft_sync_admission()
    }

    /// Returns whether PBFT startup replay has been published complete.
    pub fn is_ready(&self) -> bool {
        self.readiness.is_ready()
    }

    /// Returns whether PBFT startup finalization guard has been cleared.
    ///
    /// The service is considered ready when the PBFT head size does not exceed
    /// the delayed FinalChain finalization height (`final_chain_last_block +
    /// delegation_delay`). The calculation propagates overflow instead of
    /// wrapping, returning an explicit error code for callers.
    pub fn finalization_ready(&self, final_chain: &FinalChain) -> Result<bool> {
        let ready_height = final_chain
            .last_block_number()?
            .checked_add(final_chain.dpos_delegation_delay())
            .ok_or(anyhow!(
                "PBFT_MANAGER_FINALIZATION_WAIT_READY_HEIGHT_OVERFLOW"
            ))?;
        Ok(self.pbft_chain_head().size <= ready_height)
    }

    /// Returns whether pillar startup restoration has been published complete.
    ///
    /// The result is an acquire-load of the pillar sibling's monotonic readiness
    /// flag. It does not lock pillar state and cannot fail.
    pub fn pillar_is_ready(&self) -> bool {
        self.pillar().is_ready()
    }

    /// Verifies that live pillar work may enter the native serialization domain.
    ///
    /// This preflight exists for adapters that must preserve readiness and lock
    /// failure precedence before decoding an external operation tag. It returns
    /// no state or generation and never holds a guard across the caller boundary.
    pub fn ensure_pillar_available(&self) -> Result<()> {
        self.pillar().sample_anchor_generation().map(|_| ())
    }

    /// Publishes completed pillar bootstrap after proving state lockability.
    ///
    /// Success performs the monotonic readiness transition. A poisoned pillar
    /// lock returns an error and leaves the service pending.
    pub fn complete_pillar_bootstrap(&self) -> Result<()> {
        self.pillar().complete_bootstrap()
    }

    /// Persists and publishes canonical current-pillar data for one generation.
    ///
    /// `data_rlp` must decode canonically and `expected_anchor_generation` must
    /// match the live anchor. Persistence precedes publication; stale, malformed,
    /// overflow, readiness, and storage failures leave the snapshot unchanged.
    pub fn apply_pillar_current_block_data_for_generation(
        &self,
        data_rlp: Vec<u8>,
        expected_anchor_generation: u64,
    ) -> Result<()> {
        self.pillar()
            .apply_planned_current_block_data(data_rlp, expected_anchor_generation)
    }

    /// Persists one canonical own-pillar vote for startup recovery.
    ///
    /// Live readiness and a non-empty payload are required. Storage failures are
    /// returned without changing pillar vote aggregation state.
    pub fn apply_own_pillar_vote(&self, vote_rlp: Vec<u8>) -> Result<()> {
        self.pillar().apply_own_vote(vote_rlp)
    }

    /// Loads the durable inputs needed to reconstruct the pillar manager shell.
    ///
    /// This task accepts pending readiness. Missing rows become empty byte
    /// vectors; malformed state, period overflow, and storage failures propagate.
    pub fn load_pillar_startup_bootstrap(&self) -> Result<PillarChainStartupBootstrap> {
        self.pillar().load_startup_bootstrap()
    }

    /// Plans one current-anchor decision from the live native snapshot.
    ///
    /// `request` is already validated native input. The result carries the plan,
    /// sampled anchor, and generation; readiness or lock failures are returned.
    pub fn plan_pillar_current_anchor_decision(
        &self,
        request: PillarCurrentAnchorDecisionRequest,
    ) -> Result<PillarCurrentAnchorDecisionResult> {
        self.pillar().plan_current_anchor_decision(request)
    }

    /// Plans one pillar block using the authoritative FinalChain validator snapshot.
    ///
    /// The method samples the pillar generation, releases its lock for the
    /// FinalChain query at `request.pillar_block_period`, then re-locks and
    /// rejects generation drift. It returns the native creation plan plus ordered
    /// current counts and deltas; query, readiness, arithmetic, and stale errors
    /// propagate without publishing pillar state.
    pub fn plan_pillar_block_creation_with_final_chain(
        &self,
        final_chain: &FinalChain,
        request: PillarBlockCreationRequest,
    ) -> Result<PillarBlockCreationWithVoteCountsPlan> {
        let generation = self.pillar().sample_anchor_generation()?;
        let current_vote_counts = final_chain
            .dpos_validators_eligible_vote_counts(request.pillar_block_period.into())?
            .into_iter()
            .map(|value| PillarValidatorVoteCount {
                address: value.address.into(),
                vote_count: value.vote_count,
            })
            .collect();
        self.pillar()
            .plan_block_creation_for_generation(request, current_vote_counts, generation)
    }

    /// Validates candidate pillar linkage against native finalized state.
    ///
    /// Missing state and deterministic mismatches are represented in the plan's
    /// status. Readiness, lock, and arithmetic failures are returned as errors.
    pub fn plan_pillar_block_linkage(
        &self,
        request: PillarBlockLinkageRequest,
    ) -> Result<PillarBlockLinkagePlan> {
        self.pillar().plan_block_linkage(request)
    }

    /// Returns canonical latest-finalized pillar bytes for public materialization.
    ///
    /// Missing finalized state returns an empty vector. Live readiness and lock
    /// failures are errors; returned bytes are cloned from the validated snapshot.
    pub fn latest_finalized_pillar_block_rlp(&self) -> Result<Vec<u8>> {
        self.pillar().latest_finalized_block_rlp()
    }

    /// Validates one canonical pillar vote against live FinalChain eligibility.
    ///
    /// Preparation and cleanup remain generation-bound, and no pillar lock is
    /// held during the FinalChain query. Deterministic rejection is returned as a
    /// typed plan; decoding, lock, or FinalChain infrastructure failures are errors.
    pub fn validate_single_pillar_vote_with_final_chain(
        &self,
        final_chain: &FinalChain,
        vote_rlp: Vec<u8>,
        context: PillarVoteSingleAdmissionContext,
    ) -> Result<PillarVoteSingleAdmissionValidationPlan> {
        self.pillar()
            .pbft_service_pillar_validate_single_vote_with_final_chain(
                final_chain,
                vote_rlp,
                context,
            )
    }

    /// Applies one incoming pillar vote through FinalChain-weighted preparation.
    ///
    /// Checked ingress uses `context`; trusted local/restart ingress is selected
    /// explicitly. The method releases the pillar lock for weight/threshold reads
    /// and consumes only the matching preparation on relock. Rejections remain
    /// typed results, while decoding, lock, and infrastructure failures are errors.
    pub fn apply_single_pillar_vote_with_final_chain(
        &self,
        final_chain: &FinalChain,
        vote_rlp: Vec<u8>,
        context: PillarVoteSingleAdmissionContext,
        trusted_local_or_restore: bool,
    ) -> Result<PillarVoteSingleAdmissionWithFinalChainPlan> {
        self.pillar()
            .pbft_service_pillar_apply_single_vote_with_final_chain(
                final_chain,
                vote_rlp,
                context,
                trusted_local_or_restore,
            )
    }

    /// Resolves a pillar strict-majority threshold from FinalChain totals.
    ///
    /// `period` selects the DPoS snapshot. Future or unavailable snapshots are
    /// represented by `available == false`; pillar readiness/lock failure is an error.
    pub fn pillar_consensus_threshold_with_final_chain(
        &self,
        final_chain: &FinalChain,
        period: u64,
    ) -> Result<PillarConsensusThresholdLookup> {
        self.pillar()
            .pbft_service_pillar_consensus_threshold_with_final_chain(final_chain, period)
    }

    /// Evaluates one canonical pillar vote's relevance against live anchor state.
    ///
    /// The typed result preserves deterministic status and relevance. Readiness,
    /// lock, or malformed-input failures follow the native pillar task contract.
    pub fn plan_pillar_vote_relevance(
        &self,
        vote_rlp: Vec<u8>,
        context: PillarVoteRuntimeRelevanceContext,
    ) -> Result<PillarVoteRelevancePlan> {
        self.pillar()
            .pbft_service_pillar_plan_vote_relevance(vote_rlp, context)
    }

    /// Applies one synced pillar-vote bundle using FinalChain voting weights.
    ///
    /// Canonical inspection is generation-bound, the pillar lock is released for
    /// ordered total/validator queries, and apply revalidates the generation.
    /// Deterministic missing/zero-weight outcomes are typed; infrastructure errors
    /// never mutate aggregation state.
    pub fn apply_pillar_vote_bundle_with_final_chain(
        &self,
        final_chain: &FinalChain,
        vote_rlps: Vec<PillarVoteRlpPayload>,
        required_votes_period: u64,
    ) -> Result<PillarVoteBundleWithFinalChainPlan> {
        self.pillar()
            .pbft_service_pillar_apply_rlp_bundle_with_final_chain(
                final_chain,
                vote_rlps,
                required_votes_period,
            )
    }

    /// Returns verified pillar-vote payloads from live or persisted native state.
    ///
    /// `period`, `block_hash`, and `above_threshold` select the exact ordered
    /// payload set. Missing data is represented by the lookup; readiness, decode,
    /// storage, or lock failures are returned.
    pub fn pillar_verified_vote_payloads(
        &self,
        period: u64,
        block_hash: &[u8; 32],
        above_threshold: bool,
    ) -> Result<PillarVotesPayloadLookup> {
        self.pillar()
            .pbft_service_pillar_get_verified_vote_payloads(period, block_hash, above_threshold)
    }

    /// Generates one canonical weighted PBFT vote after compositional FinalChain reads.
    ///
    /// The service validates the vote period conversion (`period - 1`) before any
    /// state lookup and preserves the legacy C++ error strings for unavailable
    /// FinalChain rows. This method returns `Result::Err` for malformed
    /// wallets, malformed proofs, and explicit FinalChain-query failures; zero
    /// stake/total cases remain returned as typed rejected payloads. Voter stake
    /// is always read before total stake, and zero voter stake skips the total
    /// lookup. No PBFT service lock is held while FinalChain is queried.
    pub fn generate_signed_vote_with_weight(
        &self,
        final_chain: &FinalChain,
        input: PbftVoteGenerationInput,
        committee_size: u64,
        number_of_proposers: u64,
    ) -> Result<PbftGeneratedVote> {
        let dpos_period = input
            .period
            .checked_sub(1)
            .ok_or_else(|| anyhow::anyhow!("PBFT_VOTE_GENERATION_PERIOD_UNDERFLOW"))?;
        let voter = input.expected_voter;

        let voter_dpos_vote_count =
            match final_chain.pbft_dpos_eligible_vote_count(dpos_period, voter.0) {
                Ok(Some(votes)) => votes,
                Ok(None) => anyhow::bail!("PBFT_FINAL_CHAIN_ADDRESS_FACT_FUTURE_PERIOD"),
                Err(err) => {
                    anyhow::bail!("PBFT_FINAL_CHAIN_ADDRESS_FACT_UNAVAILABLE: {err}")
                }
            };

        if voter_dpos_vote_count == 0 {
            return Ok(generate_pbft_vote_with_weight(
                input,
                PbftVoteWeightFacts {
                    voter_dpos_vote_count,
                    total_dpos_vote_count: 0,
                    committee_size,
                    number_of_proposers,
                },
            )?);
        }

        let total_dpos_vote_count =
            match final_chain.pbft_dpos_eligible_total_vote_count(dpos_period) {
                Ok(Some(total)) => total,
                Ok(None) => {
                    anyhow::bail!("PBFT_FINAL_CHAIN_TOTAL_VOTES_FACT_FUTURE_PERIOD")
                }
                Err(err) => {
                    anyhow::bail!("PBFT_FINAL_CHAIN_TOTAL_VOTES_FACT_UNAVAILABLE: {err}")
                }
            };

        generate_pbft_vote_with_weight(
            input,
            PbftVoteWeightFacts {
                voter_dpos_vote_count,
                total_dpos_vote_count,
                committee_size,
                number_of_proposers,
            },
        )
    }

    /// Validates local proposer sortition identity/keys and resolves DPoS facts.
    ///
    /// Request validation happens before any FinalChain read. The method resolves
    /// DPoS facts in the original bridge order and returns typed status results
    /// when lookup outcomes are future/invalid. Identity and VRF errors precede
    /// period conversion; infrastructure lookup failures are returned as errors.
    pub fn generate_and_validate_proposer_sortition(
        &self,
        final_chain: &FinalChain,
        request: PbftProposerSortitionRequest,
    ) -> Result<PbftProposerSortitionResult> {
        let request = prepare_and_validate_pbft_proposer_sortition_request(request)?;
        let dpos_period = request
            .request
            .pbft_period
            .checked_sub(1)
            .ok_or_else(|| anyhow::anyhow!("PBFT_PROPOSER_SORTITION_PERIOD_UNDERFLOW"))?;

        let voter_dpos_vote_count = match final_chain
            .pbft_dpos_eligible_vote_count(dpos_period, request.request.expected_voter.0)
        {
            Ok(Some(votes)) => votes,
            Ok(None) => {
                return Ok(PbftProposerSortitionResult::rejected(
                    crate::pbft_vote_validation::PbftProposerSortitionStatus::FutureDposState,
                ));
            }
            Err(err) => {
                anyhow::bail!("PBFT_FINAL_CHAIN_ADDRESS_FACT_UNAVAILABLE: {err}")
            }
        };

        if voter_dpos_vote_count == 0 {
            return Ok(PbftProposerSortitionResult::rejected(
                crate::pbft_vote_validation::PbftProposerSortitionStatus::ZeroStake,
            ));
        }

        let total_dpos_vote_count =
            match final_chain.pbft_dpos_eligible_total_vote_count(dpos_period) {
                Ok(Some(total)) => total,
                Ok(None) => {
                    return Ok(PbftProposerSortitionResult::rejected(
                        crate::pbft_vote_validation::PbftProposerSortitionStatus::FutureDposState,
                    ));
                }
                Err(err) => {
                    anyhow::bail!("PBFT_FINAL_CHAIN_TOTAL_VOTES_FACT_UNAVAILABLE: {err}")
                }
            };

        generate_and_validate_proposer_sortition_with_prepared_request(
            request,
            voter_dpos_vote_count,
            total_dpos_vote_count,
        )
    }

    /// Collects one PBFT-period total DPoS-vote fact through FinalChain.
    ///
    /// The current FinalChain head is sampled first as a diagnostic value; it is
    /// not an atomic snapshot with the subsequent period lookup. Ready totals are
    /// typed data, while future or failed/corrupt lookups are typed unavailable
    /// facts. Only failure to sample the diagnostic head returns an error.
    ///
    /// The sampled head is returned for diagnostics and is not atomically tied to
    /// the sub-reads for `request.period`.
    pub fn collect_dpos_total_vote_count(
        &self,
        final_chain: &FinalChain,
        request: PbftFinalChainDposTotalVoteCountRequest,
    ) -> Result<PbftFinalChainDposTotalVoteCountFacts> {
        let last_block_number = final_chain.last_block_number_typed()?;
        let status = match final_chain.pbft_dpos_eligible_total_vote_count(request.period) {
            Ok(Some(total_vote_count)) => PbftFinalChainFact::Ready(total_vote_count),
            Ok(None) => PbftFinalChainFact::Unavailable {
                error_code: "PBFT_FINAL_CHAIN_TOTAL_VOTES_FUTURE_PERIOD".to_string(),
            },
            Err(err) => PbftFinalChainFact::Unavailable {
                error_code: format!("PBFT_FINAL_CHAIN_TOTAL_VOTES_UNAVAILABLE: {err}"),
            },
        };

        Ok(PbftFinalChainDposTotalVoteCountFacts {
            status,
            last_block_number,
        })
    }

    /// Collects one ordered wallet-subset aggregate DPoS vote fact through
    /// FinalChain.
    ///
    /// The head is sampled first for diagnostics. Addresses retain caller order
    /// and duplicates, and counts use legacy wrapping `u64` addition.
    ///
    /// Before any FinalChain address lookup, `eligible_wallet_period` must match
    /// the requested `period`. A mismatch is a deterministic, non-error short
    /// circuit with typed unavailable status and `eligible_wallet_period_ready =
    /// false`. When ready, an empty subset returns ready zero without a period
    /// lookup.
    ///
    /// Future or failed/corrupt lookups are typed unavailable facts; head-sampling
    /// failure is an error.
    ///
    /// The sampled head is returned for diagnostics and is not atomically tied to
    /// the sub-reads for this request's address list.
    pub fn collect_dpos_wallet_aggregate_vote_count(
        &self,
        final_chain: &FinalChain,
        request: PbftFinalChainDposWalletAggregateVoteCountRequest,
    ) -> Result<PbftFinalChainDposWalletAggregateVoteCountFacts> {
        let last_block_number = final_chain.last_block_number_typed()?;
        let addresses: Vec<[u8; 20]> = request.addresses.iter().map(|address| address.0).collect();
        let status = if request.eligible_wallet_period != request.period {
            PbftFinalChainFact::Unavailable {
                error_code: "PBFT_FINAL_CHAIN_WALLET_AGGREGATE_PERIOD_MISMATCH".to_string(),
            }
        } else if addresses.is_empty() {
            PbftFinalChainFact::Ready(0)
        } else {
            match final_chain.pbft_dpos_eligible_wallet_vote_counts(request.period, &addresses) {
                Ok(Some(votes)) => {
                    let aggregate_vote_count = votes
                        .iter()
                        .fold(0_u64, |total, vote| total.wrapping_add(vote.vote_count));
                    PbftFinalChainFact::Ready(aggregate_vote_count)
                }
                Ok(None) => PbftFinalChainFact::Unavailable {
                    error_code: "PBFT_FINAL_CHAIN_WALLET_VOTES_FUTURE_PERIOD".to_string(),
                },
                Err(err) => PbftFinalChainFact::Unavailable {
                    error_code: format!("PBFT_FINAL_CHAIN_WALLET_VOTES_UNAVAILABLE: {err}"),
                },
            }
        };

        Ok(PbftFinalChainDposWalletAggregateVoteCountFacts {
            status,
            last_block_number,
            eligible_wallet_period_ready: request.eligible_wallet_period == request.period,
        })
    }

    /// Collects one-wallet DPoS eligibility through FinalChain.
    ///
    /// The head is sampled first for diagnostics, then the exact period/address
    /// lookup returns a typed vote count. Eligibility is derived from a ready
    /// nonzero count. Future or failed/corrupt lookups are typed unavailable;
    /// head-sampling failure is returned as an error.
    ///
    /// The sampled head is returned for diagnostics and is not atomically tied to
    /// the lookup row for `request.address`.
    pub fn collect_dpos_wallet_eligibility(
        &self,
        final_chain: &FinalChain,
        request: PbftFinalChainDposWalletEligibilityRequest,
    ) -> Result<PbftFinalChainDposWalletEligibilityFacts> {
        let last_block_number = final_chain.last_block_number_typed()?;
        let status =
            match final_chain.pbft_dpos_eligible_vote_count(request.period, request.address.0) {
                Ok(Some(vote_count)) => PbftFinalChainFact::Ready(vote_count),
                Ok(None) => PbftFinalChainFact::Unavailable {
                    error_code: "PBFT_FINAL_CHAIN_ADDRESS_FACT_FUTURE_PERIOD".to_string(),
                },
                Err(err) => PbftFinalChainFact::Unavailable {
                    error_code: format!("PBFT_FINAL_CHAIN_ADDRESS_FACT_UNAVAILABLE: {err}"),
                },
            };

        Ok(PbftFinalChainDposWalletEligibilityFacts {
            status,
            last_block_number,
            address: request.address,
        })
    }

    /// Collects one batch of ordered one-wallet DPoS eligibility facts.
    ///
    /// The diagnostic head is sampled first. Address order and duplicates are
    /// preserved; each lookup has its own typed outcome, and the first unavailable
    /// diagnostic becomes the top-level batch error. Empty batches are ready even
    /// for future periods. Only head-sampling failure returns an error.
    ///
    /// The sampled head is returned for diagnostics and is not atomically tied to
    /// all per-address lookups.
    pub fn collect_dpos_wallet_eligibility_batch(
        &self,
        final_chain: &FinalChain,
        request: PbftFinalChainDposWalletEligibilityBatchRequest,
    ) -> Result<PbftFinalChainDposWalletEligibilityBatchFacts> {
        let last_block_number = final_chain.last_block_number_typed()?;
        let mut address_facts = Vec::with_capacity(request.addresses.len());
        let mut top_error_code = None;

        for address in request.addresses {
            let status = match final_chain.pbft_dpos_eligible_vote_count(request.period, address.0)
            {
                Ok(Some(vote_count)) => PbftFinalChainFact::Ready(vote_count),
                Ok(None) => PbftFinalChainFact::Unavailable {
                    error_code: "PBFT_FINAL_CHAIN_ADDRESS_FACT_FUTURE_PERIOD".to_string(),
                },
                Err(err) => PbftFinalChainFact::Unavailable {
                    error_code: format!("PBFT_FINAL_CHAIN_ADDRESS_FACT_UNAVAILABLE: {err}"),
                },
            };

            if top_error_code.is_none()
                && let PbftFinalChainFact::Unavailable { error_code } = &status
            {
                top_error_code = Some(error_code.clone());
            }

            address_facts.push(PbftFinalChainDposAddressVoteFact { address, status });
        }

        let status = if let Some(error_code) = top_error_code {
            PbftFinalChainFact::Unavailable { error_code }
        } else {
            PbftFinalChainFact::Ready(())
        };

        Ok(PbftFinalChainDposWalletEligibilityBatchFacts {
            status,
            last_block_number,
            address_facts,
        })
    }

    /// Prepares one pillar block for PBFT finalization under a one-shot token.
    ///
    /// The result contains deterministic request/emit effects, selected votes,
    /// canonical bytes, anchor generation, and token. No persistence occurs;
    /// readiness, decode, storage-fallback, and lock failures are returned.
    pub fn prepare_pillar_block_finalization(
        &self,
        request: PillarBlockFinalizationRequest,
    ) -> Result<PillarBlockFinalizationPrepareResult> {
        self.pillar()
            .pbft_service_pillar_prepare_finalized_block_for_pbft(request)
    }

    /// Acknowledges one prepared pillar finalization after external persistence.
    ///
    /// The request must match both anchor generation and one-shot preparation
    /// token. Success publishes finalized state and cleanup. Stale generations,
    /// missing/reused tokens, and lock failures are errors; persistence failures
    /// preserve the preparation so the same token can be retried.
    pub fn acknowledge_pillar_block_finalization(
        &self,
        request: PillarBlockFinalizationAcknowledgeRequest,
    ) -> Result<PillarBlockFinalizationAcknowledgeResult> {
        self.pillar()
            .pbft_service_pillar_ack_finalize_block_for_pbft(request)
    }

    /// Validates one canonical PBFT vote through Rust-owned FinalChain composition.
    ///
    /// The validation flow preserves bridge-compatible status precedence:
    /// voter DPoS read first, VRF key lookup second, and total DPoS read last.
    /// Replay-cache marking is applied only to the terminal validation decision and
    /// never while FinalChain lookups are in flight.
    ///
    /// `canonical_vote_rlp` is the signed vote payload and `request` contains
    /// immutable validation policy. The result contains the canonical validation,
    /// replay-publication outcome, and an optional weighted payload ready for the
    /// compatibility boundary. FinalChain lookup failures become terminal
    /// `UnknownError` validations; decode, payload construction, and vote-lock
    /// failures return an error.
    pub fn validate_verified_vote_with_final_chain(
        &self,
        final_chain: &FinalChain,
        canonical_vote_rlp: &[u8],
        request: PbftVoteAdmissionValidationRequest,
    ) -> Result<(
        PbftCanonicalVoteValidation,
        PbftVoteRuntimeReplayOutcome,
        Option<Vec<u8>>,
    )> {
        let (validation, replay) = self.validate_verified_vote_with_final_chain_internal(
            final_chain,
            canonical_vote_rlp,
            request,
            true,
        )?;
        let weighted_vote_rlp = (validation.accepted && validation.weight_calculated)
            .then(|| {
                build_weighted_pbft_vote_payload(canonical_vote_rlp, validation.calculated_weight)
                    .map(|payload| payload.vote_rlp)
            })
            .transpose()?;
        Ok((validation, replay, weighted_vote_rlp))
    }

    /// Validates and persists one canonical PBFT vote through Rust-owned FinalChain
    /// composition.
    ///
    /// Replay-cache marking is performed by the admission transaction so that
    /// validation failures and persistence rejections keep one deterministic
    /// replay mark per validation decision.
    ///
    /// The supplied canonical payload, validation policy, event flags, and
    /// progress context are consumed as one operation. The result pairs the
    /// terminal validation with the transactional admission report. Required
    /// writes commit before runtime publication; a rejected write restores the
    /// bounded runtime checkpoint and exposes no executor effects.
    pub fn admit_and_persist_verified_vote_with_final_chain(
        &self,
        final_chain: &FinalChain,
        canonical_vote_rlp: &[u8],
        request: PbftVoteAdmissionValidationRequest,
        flags: PbftVoteEventFactFlags,
        context: PbftVoteProgressContext,
        slashing_submitters: &[SlashingSubmitterIdentity],
    ) -> Result<PbftVoteAdmissionWithSlashingResult> {
        let (validation, _) = self.validate_verified_vote_with_final_chain_internal(
            final_chain,
            canonical_vote_rlp,
            request,
            false,
        )?;
        self.admit_validated_vote_with_slashing_resolver(
            canonical_vote_rlp,
            &validation,
            flags,
            context,
            None,
            || resolve_slashing_submitter_facts(final_chain, slashing_submitters),
        )
    }

    /// Validates and persists one vote using slashing account facts resolved at
    /// the retained external-EVM boundary.
    ///
    /// FinalChain remains authoritative for DPoS vote validation. The supplied
    /// facts carry only wallet order, nonce, and balance for slashing submission;
    /// malformed vote data, validation failures, and storage errors remain
    /// terminal without publishing a partial admission transition.
    pub fn admit_and_persist_verified_vote_with_external_slashing_facts(
        &self,
        final_chain: &FinalChain,
        canonical_vote_rlp: &[u8],
        request: PbftVoteAdmissionValidationRequest,
        flags: PbftVoteEventFactFlags,
        context: PbftVoteProgressContext,
        submitters: &[SlashingSubmitterIdentity],
    ) -> Result<PbftVoteAdmissionWithSlashingResult> {
        let (validation, _) = self.validate_verified_vote_with_final_chain_internal(
            final_chain,
            canonical_vote_rlp,
            request,
            false,
        )?;
        self.admit_validated_vote_with_slashing_resolver(
            canonical_vote_rlp,
            &validation,
            flags,
            context,
            None,
            || {
                Ok(submitters
                    .iter()
                    .map(|identity| SlashingSubmitterFact {
                        wallet_index: identity.wallet_index,
                        nonce: identity.nonce,
                        balance: identity.balance,
                    })
                    .collect())
            },
        )
    }

    /// Admits one locally generated vote and persists its own-vote row atomically.
    ///
    /// FinalChain-backed validation derives the authoritative weight. For a new
    /// accepted vote, the canonical weighted own-vote row and every progress
    /// write share one native batch while the verified-vote checkpoint remains
    /// reversible. Any append, lock, or commit failure rejects the publication
    /// and restores replay, admission, and retained weighted-payload state.
    /// Validation rejection and duplicate admission preserve their ordinary
    /// typed outcomes and do not write an own-vote row.
    pub fn admit_and_persist_local_generated_vote(
        &self,
        final_chain: &FinalChain,
        canonical_vote_rlp: &[u8],
        request: PbftVoteAdmissionValidationRequest,
        flags: PbftVoteEventFactFlags,
        context: PbftVoteProgressContext,
        submitters: &[SlashingSubmitterIdentity],
    ) -> Result<PbftVoteAdmissionWithSlashingResult> {
        let (validation, _) = self.validate_verified_vote_with_final_chain_internal(
            final_chain,
            canonical_vote_rlp,
            request,
            false,
        )?;
        let own_vote = if validation.accepted && validation.weight_calculated {
            let payload =
                build_weighted_pbft_vote_payload(canonical_vote_rlp, validation.calculated_weight)?;
            Some(PbftVoteStorageRecord {
                hash: payload.hash,
                vote_rlp: payload.vote_rlp,
            })
        } else {
            None
        };
        self.admit_validated_vote_with_slashing_resolver(
            canonical_vote_rlp,
            &validation,
            flags,
            context,
            own_vote,
            || {
                Ok(submitters
                    .iter()
                    .map(|identity| SlashingSubmitterFact {
                        wallet_index: identity.wallet_index,
                        nonce: identity.nonce,
                        balance: identity.balance,
                    })
                    .collect())
            },
        )
    }

    /// Publishes one validated vote transition and resolves slashing accounts
    /// only when that transition proves a double-vote conflict.
    ///
    /// Ordinary and duplicate-hash admissions never sample account state, so
    /// transient FinalChain account-snapshot lag cannot reject a valid packet.
    /// Conflict publication precedes resolution; an unavailable account view
    /// therefore returns an infrastructure error without fabricating a proof
    /// transaction or rolling back the deterministic conflict observation.
    fn admit_validated_vote_with_slashing_resolver(
        &self,
        canonical_vote_rlp: &[u8],
        validation: &PbftCanonicalVoteValidation,
        flags: PbftVoteEventFactFlags,
        context: PbftVoteProgressContext,
        own_vote: Option<PbftVoteStorageRecord>,
        resolve_submitters: impl FnOnce() -> Result<Vec<SlashingSubmitterFact>>,
    ) -> Result<PbftVoteAdmissionWithSlashingResult> {
        let transaction = if let Some(own_vote) = own_vote {
            self.verified_votes()
                .lock()?
                .admit_validated_local_vote_transactional(
                    canonical_vote_rlp,
                    validation,
                    flags,
                    context,
                    own_vote,
                    |write| persist_local_vote_admission(self.verified_votes().storage(), write),
                )?
        } else {
            self.verified_votes()
                .lock()?
                .admit_validated_vote_transactional(
                    canonical_vote_rlp,
                    validation,
                    flags,
                    context,
                    |write| persist_pbft_vote_progress(self.verified_votes().storage(), write),
                )?
        };

        let slashing_transaction_effect = if transaction.transition_published {
            transaction
                .outcome
                .slashing_payloads
                .as_ref()
                .map(|payloads| {
                    let submitters = resolve_submitters()?;
                    let progress_fact = transaction
                        .outcome
                        .precheck
                        .progress_fact
                        .as_ref()
                        .ok_or_else(|| {
                            anyhow::anyhow!("PBFT_SERVICE_SLASHING_CONFLICT_MISSING_PROGRESS_FACT")
                        })?;
                    self.slashing
                        .plan_double_voting_proof(DoubleVotingProofInput {
                            vote_a_hash: payloads.incoming.hash,
                            vote_b_hash: payloads.conflicting.hash,
                            vote_a_period: progress_fact.identity.period,
                            vote_b_period: progress_fact.identity.period,
                            vote_a_round: progress_fact.identity.round,
                            vote_b_round: progress_fact.identity.round,
                            vote_a_step: progress_fact.identity.step,
                            vote_b_step: progress_fact.identity.step,
                            vote_a_rlp: payloads.incoming.vote_rlp.clone(),
                            vote_b_rlp: payloads.conflicting.vote_rlp.clone(),
                            submitters,
                        })
                        .map(Into::into)
                })
                .transpose()?
        } else {
            None
        };

        Ok(PbftVoteAdmissionWithSlashingResult {
            validation: validation.clone(),
            transaction,
            slashing_transaction_effect,
        })
    }

    /// Computes one PBFT `2t+1` threshold and resolves total eligible DPoS
    /// through Rust-owned FinalChain composition when required.
    ///
    /// `fact` supplies only period, vote type, and committee policy; caller-owned
    /// chain or DPoS values are overwritten. A cache hit returns without a
    /// FinalChain lookup. A miss samples total stake without either the chain or
    /// vote lock held, refreshes the chain head, and returns the typed planner
    /// result; missing future state and lookup errors remain fail-closed statuses.
    pub fn verified_votes_two_t_plus_one_threshold_with_final_chain(
        &self,
        final_chain: &FinalChain,
        mut fact: PbftTwoTPlusOneThresholdFact,
    ) -> Result<PbftTwoTPlusOneThresholdPlan> {
        fact.has_total_dpos_votes_count = false;
        fact.total_dpos_votes_count = 0;
        fact.future_dpos_state = false;
        fact.unknown_error = false;
        fact.current_pbft_chain_size = self
            .chain()
            .read()
            .map_err(|_| anyhow::anyhow!("PBFT_SERVICE_CHAIN_LOCK_POISONED"))?
            .state
            .head()
            .size;

        let initial = {
            let mut votes = self.verified_votes().lock()?;
            votes.plan_two_t_plus_one_threshold(fact)
        };
        if !initial.needs_total_dpos_votes {
            return Ok(initial);
        }

        match final_chain.pbft_dpos_eligible_total_vote_count(fact.pbft_period) {
            Ok(Some(total)) => {
                fact.has_total_dpos_votes_count = true;
                fact.total_dpos_votes_count = total;
            }
            Ok(None) => fact.future_dpos_state = true,
            Err(_) => fact.unknown_error = true,
        }

        fact.current_pbft_chain_size = self
            .chain()
            .read()
            .map_err(|_| anyhow::anyhow!("PBFT_SERVICE_CHAIN_LOCK_POISONED"))?
            .state
            .head()
            .size;

        let resolved = {
            let mut votes = self.verified_votes().lock()?;
            votes.plan_two_t_plus_one_threshold(fact)
        };
        Ok(resolved)
    }

    /// Resolves a public-client PBFT quorum without exposing configuration or mutable vote state.
    ///
    /// The application query facade supplies only the semantic period and vote
    /// kind. Committee policy, the live chain head, cached totals, and exact
    /// FinalChain DPoS state are composed here. The returned planner preserves
    /// typed future-state and infrastructure failure statuses.
    pub fn public_vote_threshold(
        &self,
        final_chain: &FinalChain,
        period: u64,
        vote_type: PbftVoteType,
    ) -> Result<PbftTwoTPlusOneThresholdPlan> {
        self.verified_votes_two_t_plus_one_threshold_with_final_chain(
            final_chain,
            PbftTwoTPlusOneThresholdFact {
                pbft_period: period,
                vote_type,
                current_pbft_chain_size: 0,
                committee_size: self.committee_size,
                number_of_proposers: self.number_of_proposers,
                has_total_dpos_votes_count: false,
                total_dpos_votes_count: 0,
                future_dpos_state: false,
                unknown_error: false,
            },
        )
    }

    /// Validates, persists, and publishes one network-ingress PBFT vote.
    ///
    /// Packet routing supplies only canonical vote bytes and ordered signing
    /// identities. This root task derives the live manager cursor, committee
    /// policy, quorum threshold, and slashing policy from the restored PBFT
    /// service. FinalChain remains the authoritative DPoS/account boundary.
    /// A published double-vote returns a typed transaction effect; signing and
    /// transaction insertion remain external and must be reported separately.
    pub fn admit_network_verified_vote(
        &self,
        final_chain: &FinalChain,
        canonical_vote_rlp: &[u8],
        slashing_submitters: &[SlashingSubmitterIdentity],
    ) -> Result<PbftVoteAdmissionWithSlashingResult> {
        let inspection = inspect_canonical_pbft_vote(canonical_vote_rlp)?;
        let manager = self.manager_snapshot();
        let threshold = inspection.period.checked_sub(1).and_then(|period| {
            self.public_vote_threshold(final_chain, period, inspection.vote_type)
                .ok()
                .filter(|plan| {
                    plan.status == PbftTwoTPlusOneThresholdStatus::Available && plan.has_threshold
                })
                .map(|plan| plan.threshold)
        });
        self.admit_and_persist_verified_vote_with_final_chain(
            final_chain,
            canonical_vote_rlp,
            PbftVoteAdmissionValidationRequest {
                strict_vrf: true,
                committee_size: self.committee_size,
                number_of_proposers: self.number_of_proposers,
                has_preverified_weight: inspection.has_embedded_weight,
                preverified_weight: inspection.embedded_weight,
            },
            PbftVoteEventFactFlags {
                vote_already_known: false,
                carries_proposed_block: true,
                valid_stale_reward_vote: false,
            },
            PbftVoteProgressContext {
                current_period: manager.period,
                current_round: manager.round,
                max_future_period_delta: u64::MAX,
                two_t_plus_one_threshold: threshold,
                require_proposed_block_sidecar: false,
                slashing_enabled: self.slashing_enabled,
            },
            slashing_submitters,
        )
    }

    fn validate_verified_vote_with_final_chain_internal(
        &self,
        final_chain: &FinalChain,
        canonical_vote_rlp: &[u8],
        request: PbftVoteAdmissionValidationRequest,
        record_replay: bool,
    ) -> Result<(PbftCanonicalVoteValidation, PbftVoteRuntimeReplayOutcome)> {
        let inspection = inspect_canonical_pbft_vote(canonical_vote_rlp)?;
        let mut facts = PbftVoteValidationExternalFacts {
            voter_dpos_ready: false,
            voter_dpos_vote_count: 0,
            total_dpos_ready: false,
            total_dpos_vote_count: 0,
            future_dpos_state: false,
            unknown_error: false,
            vrf_key_ready: false,
            has_vrf_key: false,
            vrf_public_key: [0; 32],
            strict_vrf: request.strict_vrf,
            committee_size: request.committee_size,
            number_of_proposers: request.number_of_proposers,
            has_preverified_weight: request.has_preverified_weight,
            preverified_weight: request.preverified_weight,
        };

        let mut validation = validate_canonical_pbft_vote(canonical_vote_rlp, facts)?;
        if inspection.status != PbftCanonicalVoteInspectionStatus::Valid
            || request.has_preverified_weight
        {
            let replay = record_replay
                .then(|| {
                    self.verified_votes()
                        .lock()
                        .map(|mut votes| votes.record_validation_replay(&validation))
                })
                .transpose()?;
            return Ok((
                validation,
                replay.unwrap_or_else(Self::empty_replay_outcome),
            ));
        }

        let Some(dpos_period) = inspection.period.checked_sub(1) else {
            facts.unknown_error = true;
            let validation = validate_canonical_pbft_vote(canonical_vote_rlp, facts)?;
            let replay = record_replay
                .then(|| {
                    self.verified_votes()
                        .lock()
                        .map(|mut votes| votes.record_validation_replay(&validation))
                })
                .transpose()?;
            return Ok((
                validation,
                replay.unwrap_or_else(Self::empty_replay_outcome),
            ));
        };

        let voter = inspection.recovered_voter.0;
        match final_chain.pbft_dpos_eligible_vote_count(dpos_period, voter) {
            Ok(Some(votes)) => {
                facts.voter_dpos_ready = true;
                facts.voter_dpos_vote_count = votes;
            }
            Ok(None) => facts.future_dpos_state = true,
            Err(_) => facts.unknown_error = true,
        }
        validation = validate_canonical_pbft_vote(canonical_vote_rlp, facts)?;
        if validation.rejected || facts.future_dpos_state || facts.unknown_error {
            let replay = record_replay
                .then(|| {
                    self.verified_votes()
                        .lock()
                        .map(|mut votes| votes.record_validation_replay(&validation))
                })
                .transpose()?;
            return Ok((
                validation,
                replay.unwrap_or_else(Self::empty_replay_outcome),
            ));
        }

        let mut key_lookup_error = false;
        let cached_key = {
            let votes = self.verified_votes();
            let runtime = votes.lock()?;
            runtime.validation_vrf_key(voter)
        };
        if let Some(key) = cached_key {
            facts.vrf_key_ready = true;
            facts.has_vrf_key = true;
            facts.vrf_public_key = key;
        } else {
            match final_chain.pbft_vrf_key_with_fallback(dpos_period, voter) {
                Ok(Some(key)) => {
                    facts.vrf_key_ready = true;
                    facts.has_vrf_key = true;
                    facts.vrf_public_key = key;
                    self.verified_votes()
                        .lock()?
                        .cache_validation_vrf_key(voter, key);
                }
                Ok(None) => {}
                Err(_) => key_lookup_error = true,
            }
        }

        if key_lookup_error {
            facts.unknown_error = true;
        } else {
            facts.vrf_key_ready = true;
        }

        validation = validate_canonical_pbft_vote(canonical_vote_rlp, facts)?;
        if validation.rejected || facts.unknown_error {
            let replay = record_replay
                .then(|| {
                    self.verified_votes()
                        .lock()
                        .map(|mut votes| votes.record_validation_replay(&validation))
                })
                .transpose()?;
            return Ok((
                validation,
                replay.unwrap_or_else(Self::empty_replay_outcome),
            ));
        }

        match final_chain.pbft_dpos_eligible_total_vote_count(dpos_period) {
            Ok(Some(total)) => {
                facts.total_dpos_ready = true;
                facts.total_dpos_vote_count = total;
            }
            Ok(None) => facts.future_dpos_state = true,
            Err(_) => facts.unknown_error = true,
        }

        validation = validate_canonical_pbft_vote(canonical_vote_rlp, facts)?;
        let replay = record_replay
            .then(|| {
                self.verified_votes()
                    .lock()
                    .map(|mut votes| votes.record_validation_replay(&validation))
            })
            .transpose()?;
        Ok((
            validation,
            replay.unwrap_or_else(Self::empty_replay_outcome),
        ))
    }

    /// Locks the native manager serialization domain.
    pub(crate) fn manager_state(&self) -> PbftManagerGuard<'_> {
        self.manager.lock()
    }

    /// Returns the native PBFT-chain sibling.
    pub(crate) fn chain(&self) -> &PbftChainService {
        &self.chain
    }

    /// Returns the native proposed-block sibling.
    pub(crate) fn proposed_blocks(&self) -> &ProposedBlocksService {
        &self.proposed_blocks
    }

    /// Returns the native verified-vote sibling.
    pub(crate) fn verified_votes(&self) -> &PbftVerifiedVotesService {
        &self.verified_votes
    }

    /// Returns a clone of the PBFT-root-owned consensus network service.
    ///
    /// The clone shares the one native effect queue and the same restored
    /// pillar and verified-vote siblings as this root. It is safe to retain at
    /// the external transport boundary; sibling queries never execute while
    /// the network queue mutex is held. No independent network composition or
    /// configuration is created by this accessor.
    pub fn network_service(&self) -> ConsensusNetworkService {
        self.network.clone()
    }

    /// Returns verified-vote storage snapshots and sidecars through the service.
    ///
    /// Inputs, ordering, validation, and storage error behavior are exactly the
    /// native sibling contract in [`PbftVerifiedVotesService::verified_votes_own_vote_records`].
    pub fn verified_votes_own_vote_records(&self) -> Result<Vec<PbftVoteStorageRecord>> {
        self.verified_votes().verified_votes_own_vote_records()
    }

    /// Returns deterministic verified-vote count under runtime lock.
    ///
    /// The output and lock-poison behavior are exactly
    /// [`PbftVerifiedVotesService::verified_votes_size`].
    pub fn verified_votes_size(&self) -> Result<u64> {
        self.verified_votes().verified_votes_size()
    }

    /// Checks replay-protection membership for one vote hash.
    ///
    /// `vote_hash`, the boolean output, and lock errors follow
    /// [`PbftVerifiedVotesService::verified_votes_replay_contains`].
    pub fn verified_votes_replay_contains(&self, vote_hash: H256) -> Result<bool> {
        self.verified_votes()
            .verified_votes_replay_contains(vote_hash)
    }

    /// Inserts one replay-protection membership bit.
    ///
    /// The inserted/already-present result and atomicity follow
    /// [`PbftVerifiedVotesService::verified_votes_replay_insert`].
    pub fn verified_votes_replay_insert(&self, vote_hash: H256) -> Result<bool> {
        self.verified_votes()
            .verified_votes_replay_insert(vote_hash)
    }

    /// Derives the next round for a supplied period/current-round pair.
    ///
    /// `None`, coherent-read, and lock-error behavior follow
    /// [`PbftVerifiedVotesService::verified_votes_determine_new_round`].
    pub fn verified_votes_determine_new_round(
        &self,
        period: u64,
        current_round: u64,
    ) -> Result<Option<DetermineNewRoundOutcome>> {
        self.verified_votes()
            .verified_votes_determine_new_round(period, current_round)
    }

    /// Loads one voted-block mapping from runtime-owned next-vote 2t+1 state.
    ///
    /// Typed input, optional output, and error behavior follow
    /// [`PbftVerifiedVotesService::verified_votes_get_two_t_plus_one_voted_block`].
    pub fn verified_votes_get_two_t_plus_one_voted_block(
        &self,
        period: u64,
        round: u64,
        kind: TwoTPlusOneVotedBlockType,
    ) -> Result<Option<VerifiedVotesTwoTPlusOneVotedBlock>> {
        self.verified_votes()
            .verified_votes_get_two_t_plus_one_voted_block(period, round, kind)
    }

    /// Loads retained payloads for one mapped voted block.
    ///
    /// Typed input, ordered optional output, and missing-sidecar errors follow
    /// [`PbftVerifiedVotesService::verified_votes_get_two_t_plus_one_voted_block_payloads`].
    pub fn verified_votes_get_two_t_plus_one_voted_block_payloads(
        &self,
        period: u64,
        round: u64,
        kind: TwoTPlusOneVotedBlockType,
    ) -> Result<Option<VerifiedVotesTwoTPlusOneVotePayloads>> {
        self.verified_votes()
            .verified_votes_get_two_t_plus_one_voted_block_payloads(period, round, kind)
    }

    /// Returns the current reward-vote cursor snapshot.
    ///
    /// The coherent output, empty-cursor sentinel, and lock errors follow the
    /// native verified-vote sibling's reward cursor contract.
    pub fn reward_vote_cursor_snapshot(&self) -> Result<RewardVoteCursorSnapshot> {
        self.verified_votes().reward_vote_cursor_snapshot()
    }

    /// Returns the latest reset period from reward-vote cursor state.
    ///
    /// Returns zero before the first reset and propagates native sibling lock
    /// errors without exposing mutable state.
    pub fn reward_vote_period(&self) -> Result<u64> {
        self.verified_votes().reward_vote_period()
    }

    /// Selects reward-vote payloads for one cert family.
    ///
    /// `block_period` and ordered requested hashes produce the sibling's typed
    /// selection result; mapping, payload, and lock failures propagate.
    pub fn select_reward_vote_payloads(
        &self,
        block_period: u64,
        requested_vote_hashes: Vec<H256>,
    ) -> Result<PbftRewardVotePayloadSelection> {
        self.verified_votes()
            .select_reward_vote_payloads(block_period, requested_vote_hashes)
    }

    /// Builds one optimized next-vote 2t+1 bundle egress plan.
    ///
    /// Query coordinates, coherent two-family output, and errors follow
    /// [`PbftVerifiedVotesService::verified_votes_plan_next_votes_bundle_egress`].
    pub fn verified_votes_plan_next_votes_bundle_egress(
        &self,
        period: u64,
        round: u64,
    ) -> Result<PbftNextVotesBundleEgressPlan> {
        self.verified_votes()
            .verified_votes_plan_next_votes_bundle_egress(period, round)
    }

    /// Builds complete next and next-null egress payloads atomically.
    ///
    /// Both family lookups and encodings share one verified-vote lock epoch;
    /// missing families are empty and invariant failures abort the pair.
    pub fn verified_votes_build_next_votes_bundle_egress(
        &self,
        period: u64,
        round: u64,
    ) -> Result<PbftNextVotesBundleEgressPayloads> {
        self.verified_votes()
            .verified_votes_build_next_votes_bundle_egress(period, round)
    }

    /// Builds one optimized verified-vote bundle payload from retained hashes.
    ///
    /// Typed statuses, bundle bytes, ordering invariants, and errors follow
    /// [`PbftVerifiedVotesService::verified_votes_build_optimized_votes_bundle_egress`].
    pub fn verified_votes_build_optimized_votes_bundle_egress(
        &self,
        request: PbftOptimizedVoteBundleBuildRequest,
    ) -> Result<PbftOptimizedVoteBundleBuildResult> {
        self.verified_votes()
            .verified_votes_build_optimized_votes_bundle_egress(request)
    }

    /// Applies bounded verified-votes cleanup.
    ///
    /// The cutoff input, atomic runtime mutation, and lock errors follow
    /// [`PbftVerifiedVotesService::verified_votes_cleanup_votes_by_period`].
    pub fn verified_votes_cleanup_votes_by_period(&self, pbft_period: u64) -> Result<()> {
        self.verified_votes()
            .verified_votes_cleanup_votes_by_period(pbft_period)
    }

    /// Persists one latest-round own verified vote.
    ///
    /// The canonical signed bytes and authoritative weight are encoded and
    /// validated inside the verified-vote service. Storage serialization,
    /// typed rejection, and errors follow
    /// [`PbftVerifiedVotesService::verified_votes_save_own_verified_vote`].
    pub fn verified_votes_save_own_verified_vote(
        &self,
        canonical_vote_rlp: &[u8],
        weight: u64,
    ) -> Result<PbftVotePersistenceResult> {
        self.verified_votes()
            .verified_votes_save_own_verified_vote(canonical_vote_rlp, weight)
    }

    /// Clears all latest-round own verified vote records.
    ///
    /// Enumeration, batch semantics, typed rejection, and errors follow
    /// [`PbftVerifiedVotesService::verified_votes_clear_own_verified_votes`].
    pub fn verified_votes_clear_own_verified_votes(&self) -> Result<PbftVotePersistenceResult> {
        self.verified_votes()
            .verified_votes_clear_own_verified_votes()
    }

    /// Persists generated vote-progress effects.
    ///
    /// Typed vote/mapping identities, retained-payload resolution,
    /// runtime-to-storage serialization, result statuses, and errors follow
    /// [`PbftVerifiedVotesService::verified_votes_persist_pbft_vote_progress`].
    pub fn verified_votes_persist_pbft_vote_progress(
        &self,
        write: PbftVerifiedVoteProgressPersistenceWrite,
    ) -> Result<PbftVotePersistenceResult> {
        self.verified_votes()
            .verified_votes_persist_pbft_vote_progress(write)
    }

    /// Applies the full reward-vote reset pipeline.
    ///
    /// The typed request produces the sibling's storage/live-publication result;
    /// identity, monotonicity, lock, and storage failures propagate unchanged.
    pub fn apply_reward_votes_reset(
        &self,
        request: RewardVoteResetApplyRequest,
    ) -> Result<PbftFinalizedPeriodApplyResult> {
        self.verified_votes().apply_reward_votes_reset(request)
    }

    /// Returns deterministic verified-vote runtime state snapshot.
    ///
    /// Coherence, ordering, missing-sidecar invariants, and errors follow
    /// [`PbftVerifiedVotesService::verified_votes_state_snapshot`].
    pub fn verified_votes_state_snapshot(&self) -> Result<VerifiedVotesStateSnapshot> {
        self.verified_votes().verified_votes_state_snapshot()
    }

    /// Returns one step payload snapshot.
    ///
    /// Typed coordinates, `None` semantics, payload ordering, and errors follow
    /// [`PbftVerifiedVotesService::verified_votes_step_payloads`].
    pub fn verified_votes_step_payloads(
        &self,
        period: u64,
        round: u64,
        step: u64,
    ) -> Result<Option<Vec<VerifiedStepVotePayloadEntry>>> {
        self.verified_votes()
            .verified_votes_step_payloads(period, round, step)
    }

    /// Returns one deterministic reward-vote snapshot from native service state.
    ///
    /// Cursor and records are observed coherently; missing payloads and lock
    /// failures propagate from the native sibling without partial output.
    pub fn current_reward_vote_snapshot(&self) -> Result<RewardVotePayloadSnapshot> {
        self.verified_votes().current_reward_vote_snapshot()
    }

    /// Returns the native pillar sibling.
    pub(crate) fn pillar(&self) -> &PillarChainService {
        &self.pillar
    }
}

const fn classify_sync_reward_failure(
    block_period: u64,
    reward_cursor_period: u64,
) -> PbftSyncIngressAction {
    if block_period <= reward_cursor_period {
        PbftSyncIngressAction::StopSyncing
    } else {
        PbftSyncIngressAction::Malicious
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag::{DagManagerBlock, dag_manager_block_from_rlp, save_dag_block_to_storage};
    use crate::dag_service::DagServiceConfig;
    use crate::dag_transaction_service::{DagTransactionService, DagTransactionServiceConfig};
    use crate::gas_pricer::GasPricerConfig;
    use crate::network_api::{
        NETWORK_INGRESS_STATUS_PILLAR_VOTES_INACTIVE, NETWORK_INGRESS_STATUS_PILLAR_VOTES_NO_DATA,
        NetworkGetPillarVotesBundleRequest, NetworkPbftNextVotesBundleRequest,
    };
    use crate::pbft_chain::{PbftBlockValidation, PbftChainHead};
    use crate::pbft_thresholds::PbftTwoTPlusOneThresholdStatus;
    use crate::pbft_vote_event::PbftVoteEventFactFlags;
    use crate::pbft_vote_generation::{
        PbftVoteGenerationInput, PbftVoteGenerationStatus, generate_pbft_vote,
    };
    use crate::pbft_vote_progress::PbftVoteProgressContext;
    use crate::pbft_vote_validation::{
        PbftProposerSortitionRequest, PbftProposerSortitionStatus, PbftVoteValidationExternalFacts,
        PbftVoteValidationStatus, validate_canonical_pbft_vote,
    };
    use crate::period_data_queue::PeriodDataQueueEntryRef;
    use crate::sortition::{SortitionConfig, SortitionParams, VdfParams, VrfParams};
    use crate::transaction_service::TransactionServiceConfig;
    use crate::verified_votes::PbftVoteType;
    use ethereum_types::{H160, H256, U256};
    use k256::ecdsa::SigningKey;
    use rlp::{Rlp, RlpStream};
    use rustaxa_storage::{Column, Config};
    use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
    use rustaxa_types::pbft::PbftBlockLink;
    use rustaxa_types::pillar::{
        CurrentPillarBlockDataDb, PillarBlock, PillarVote, ValidatorVoteCount,
    };
    use rustaxa_vdf::vrf;
    use std::convert::TryFrom;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use tiny_keccak::{Hasher, Keccak};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    const NODE_SECRET: [u8; 32] = [0x35; 32];
    const NODE_SECRET_TWO: [u8; 32] = [0x42; 32];
    const NODE_SECRET_ZERO_STAKE: [u8; 32] = [0x55; 32];
    const VRF_SECRET: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn temp_storage(name: &str) -> (PathBuf, Arc<Storage>) {
        let path = std::env::temp_dir().join(format!(
            "{name}_{}_{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        let storage = Arc::new(Storage::new(Config::new(path.clone())).expect("storage opens"));
        (path, storage)
    }

    fn config(cacti_block: u64) -> PbftServiceConfig {
        PbftServiceConfig {
            genesis_lambda_ms: 100,
            cacti_lambda_max_ms: 1_500,
            cacti_lambda_default_ms: 500,
            cacti_block,
            max_exponential_lambda_ms: 60_000,
            max_steps: 13,
            deadline_ms: 1_000,
            polling_interval_ms: 100,
            report_malicious_behaviour: true,
            magnolia_activation_period: 0,
            ficus_activation_period: 10,
            pillar_blocks_interval: 10,
            sync_level_size: 10,
            is_light_node: false,
            light_node_history: 0,
            committee_size: 1,
            number_of_proposers: 1,
        }
    }

    #[test]
    fn sync_reward_cursor_failure_is_benign_only_at_or_behind_cursor() {
        assert_eq!(
            classify_sync_reward_failure(9, 9),
            PbftSyncIngressAction::StopSyncing
        );
        assert_eq!(
            classify_sync_reward_failure(8, 9),
            PbftSyncIngressAction::StopSyncing
        );
        assert_eq!(
            classify_sync_reward_failure(10, 9),
            PbftSyncIngressAction::Malicious
        );
    }

    #[test]
    fn slashing_submitter_resolution_stops_after_first_funded_wallet() {
        let identities = [
            SlashingSubmitterIdentity {
                wallet_index: 2,
                address: [2; 20],
                nonce: U256::zero(),
                balance: U256::zero(),
            },
            SlashingSubmitterIdentity {
                wallet_index: 7,
                address: [7; 20],
                nonce: U256::zero(),
                balance: U256::zero(),
            },
            SlashingSubmitterIdentity {
                wallet_index: 9,
                address: [9; 20],
                nonce: U256::zero(),
                balance: U256::zero(),
            },
        ];
        let mut queried = Vec::new();
        let submitters = resolve_slashing_submitter_facts_with(&identities, |identity| {
            queried.push(identity.wallet_index);
            anyhow::ensure!(identity.wallet_index != 9, "irrelevant wallet was queried");
            Ok((
                U256::from(identity.wallet_index),
                if identity.wallet_index == 7 {
                    U256::from(1)
                } else {
                    U256::zero()
                },
            ))
        })
        .unwrap();

        assert_eq!(queried, vec![2, 7]);
        assert_eq!(submitters.len(), 2);
        assert_eq!(submitters[1].wallet_index, 7);
        assert_eq!(submitters[1].nonce, U256::from(7));
    }

    #[test]
    fn sync_cert_bundle_rejects_shape_before_verified_vote_mutation() {
        let (path, storage) = temp_storage("pbft_sync_cert_shape_preflight");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        let final_chain = final_chain_with_vote_validator(
            storage,
            voter_from_secret(&NODE_SECRET),
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            5_000,
        );
        let block_hash = H256::repeat_byte(0x81);
        let wrong_period_vote = generated_vote_at_period(block_hash, 2);

        let result = service
            .begin_pbft_sync_cert_bundle(
                &final_chain,
                1,
                block_hash,
                vec![wrong_period_vote.vote_rlp],
                Vec::new(),
            )
            .unwrap();

        assert_eq!(result.action, PbftSyncCertBundleAction::Rejected);
        assert_eq!(result.status, PbftSyncCertVoteBundleStatus::PeriodMismatch);
        assert!(result.weighted_vote_rlps.is_empty());
        assert!(result.slashing_transaction_effect.is_none());
        assert_eq!(service.verified_votes_size().unwrap(), 0);

        drop(final_chain);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn local_generated_vote_task_publishes_admission_and_own_vote_together() {
        let (path, storage) = temp_storage("pbft_local_generated_vote_atomic");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        let final_chain = final_chain_with_vote_validator(
            storage.clone(),
            voter_from_secret(&NODE_SECRET),
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            5_000,
        );
        let vote = generated_vote_at_period(H256::repeat_byte(0x80), 1);

        let result = service
            .admit_and_persist_local_generated_vote(
                &final_chain,
                &vote.vote_rlp,
                vote_validation_request(false, 0),
                PbftVoteEventFactFlags {
                    vote_already_known: false,
                    carries_proposed_block: true,
                    valid_stale_reward_vote: false,
                },
                PbftVoteProgressContext {
                    current_period: 1,
                    current_round: 2,
                    max_future_period_delta: u64::MAX,
                    two_t_plus_one_threshold: None,
                    require_proposed_block_sidecar: false,
                    slashing_enabled: true,
                },
                &[],
            )
            .unwrap();

        assert!(result.validation.accepted);
        assert!(result.transaction.transition_published);
        assert_eq!(result.transaction.persistence_applied_writes, 1);
        assert_eq!(service.verified_votes_size().unwrap(), 1);
        let own_votes = storage.pbft().own_verified_votes_rlp().unwrap();
        assert_eq!(own_votes.len(), 1);
        assert_eq!(
            own_votes[0],
            build_weighted_pbft_vote_payload(&vote.vote_rlp, result.validation.calculated_weight,)
                .unwrap()
                .vote_rlp
        );

        drop(final_chain);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn sync_cert_bundle_pauses_reports_exact_effect_and_then_accepts() {
        let (path, storage) = temp_storage("pbft_sync_cert_resumable_slashing");
        let voter = voter_from_secret(&NODE_SECRET);
        let submitter = [0x78; 20];
        let service_config = config(0);
        let service = PbftService::restore(storage.clone(), service_config).unwrap();
        let final_chain = final_chain_with_vote_validator_and_account(
            storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            5_000,
            submitter,
        );
        let submitter_facts = vec![SlashingSubmitterIdentity {
            wallet_index: 0,
            address: submitter,
            nonce: U256::zero(),
            balance: U256::from(1_000_000u64),
        }];
        let conflicting = generated_vote_at_period(H256::repeat_byte(0x82), 1);
        let initial = service
            .admit_and_persist_verified_vote_with_final_chain(
                &final_chain,
                &conflicting.vote_rlp,
                vote_validation_request(false, 0),
                PbftVoteEventFactFlags {
                    vote_already_known: false,
                    carries_proposed_block: true,
                    valid_stale_reward_vote: false,
                },
                PbftVoteProgressContext {
                    current_period: 1,
                    current_round: 1,
                    max_future_period_delta: u64::MAX,
                    two_t_plus_one_threshold: Some(1),
                    require_proposed_block_sidecar: false,
                    slashing_enabled: true,
                },
                &[],
            )
            .unwrap();
        assert!(initial.validation.accepted);

        let block_hash = H256::repeat_byte(0x83);
        let incoming = generated_vote_at_period(block_hash, 1);
        let awaiting = service
            .begin_pbft_sync_cert_bundle(
                &final_chain,
                1,
                block_hash,
                vec![incoming.vote_rlp],
                submitter_facts.clone(),
            )
            .unwrap();
        assert_eq!(awaiting.action, PbftSyncCertBundleAction::AwaitingSlashing);
        assert!(awaiting.weighted_vote_rlps.is_empty());
        let effect = awaiting
            .slashing_transaction_effect
            .as_ref()
            .expect("conflict pauses for one external effect");

        assert!(
            service
                .report_pbft_sync_cert_bundle_slashing(
                    awaiting.session_id,
                    awaiting.effect_id + 1,
                    effect.proof_hash,
                    false,
                )
                .is_err()
        );
        let accepted = service
            .report_pbft_sync_cert_bundle_slashing(
                awaiting.session_id,
                awaiting.effect_id,
                effect.proof_hash,
                false,
            )
            .unwrap();
        assert_eq!(accepted.action, PbftSyncCertBundleAction::Accepted);
        assert_eq!(accepted.weighted_vote_rlps.len(), 1);
        assert!(accepted.slashing_transaction_effect.is_none());

        let duplicate = service
            .report_pbft_sync_cert_bundle_slashing(
                awaiting.session_id,
                awaiting.effect_id,
                effect.proof_hash,
                false,
            )
            .unwrap();
        assert_eq!(duplicate, accepted);

        let retry_hash = H256::repeat_byte(0x84);
        let retry_vote = generated_vote_at_period(retry_hash, 1);
        let pending = service
            .begin_pbft_sync_cert_bundle(
                &final_chain,
                1,
                retry_hash,
                vec![retry_vote.vote_rlp.clone()],
                submitter_facts.clone(),
            )
            .unwrap();
        assert_eq!(pending.action, PbftSyncCertBundleAction::AwaitingSlashing);
        assert!(
            !service
                .abort_pbft_sync_cert_bundle(pending.session_id + 1)
                .unwrap()
        );
        assert!(
            service
                .abort_pbft_sync_cert_bundle(pending.session_id)
                .unwrap()
        );
        let restarted = service
            .begin_pbft_sync_cert_bundle(
                &final_chain,
                1,
                retry_hash,
                vec![retry_vote.vote_rlp],
                submitter_facts,
            )
            .unwrap();
        assert_eq!(restarted.action, PbftSyncCertBundleAction::AwaitingSlashing);
        assert_ne!(restarted.session_id, pending.session_id);
        assert!(
            service
                .abort_pbft_sync_cert_bundle(restarted.session_id)
                .unwrap()
        );

        drop(final_chain);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    fn signed_period_one_sync_packet() -> (Vec<u8>, Vec<u8>) {
        let signing_key = SigningKey::from_slice(&[0x61; 32]).unwrap();
        let append_unsigned_fields = |stream: &mut RlpStream| {
            stream.append(&H256::zero());
            stream.append(&H256::from_low_u64_be(2));
            stream.append(&H256::zero());
            stream.append(&H256::from_low_u64_be(3));
            stream.append(&1_u64);
            stream.append(&7_u64);
            stream.begin_list(0);
        };
        let mut unsigned = RlpStream::new_list(7);
        append_unsigned_fields(&mut unsigned);
        let mut digest = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&unsigned.out());
        hasher.finalize(&mut digest);
        let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&digest).unwrap();
        let mut signature = signature.to_bytes().to_vec();
        signature.push(recovery_id.to_byte());

        let mut block = RlpStream::new_list(8);
        append_unsigned_fields(&mut block);
        block.append(&signature);
        let mut period_data = RlpStream::new_list(4);
        period_data.append_raw(&block.out(), 1);
        period_data.append_empty_data();
        period_data.append_empty_data();
        period_data.begin_list(0);
        let period_data_rlp = period_data.out().to_vec();
        let mut packet = RlpStream::new_list(3);
        packet.append(&true);
        packet.append_raw(&period_data_rlp, 1);
        packet.append_empty_data();
        (packet.out().to_vec(), period_data_rlp)
    }

    fn optimized_cert_bundle(votes: &[Vec<u8>]) -> Vec<u8> {
        let first = Rlp::new(&votes[0]);
        let block_hash: H256 = first.val_at(0).unwrap();
        let first_sortition_bytes: Vec<u8> = first.val_at(1).unwrap();
        let first_sortition = Rlp::new(&first_sortition_bytes);
        let period: u64 = first_sortition.val_at(0).unwrap();
        let round: u64 = first_sortition.val_at(1).unwrap();
        let step: u64 = first_sortition.val_at(2).unwrap();

        let mut bundle = RlpStream::new_list(5);
        bundle.append(&block_hash);
        bundle.append(&period);
        bundle.append(&round);
        bundle.append(&step);
        bundle.begin_list(votes.len());
        for vote in votes {
            let vote = Rlp::new(vote);
            let sortition_bytes: Vec<u8> = vote.val_at(1).unwrap();
            let sortition = Rlp::new(&sortition_bytes);
            let proof: Vec<u8> = sortition.val_at(3).unwrap();
            let signature: Vec<u8> = vote.val_at(2).unwrap();
            bundle.begin_list(2);
            bundle.append(&proof);
            bundle.append(&signature);
        }
        bundle.out().to_vec()
    }

    fn signed_period_two_sync_packet(
        previous_votes: &[Vec<u8>],
        reward_vote_hash: H256,
    ) -> (Vec<u8>, Vec<u8>) {
        let signing_key = SigningKey::from_slice(&[0x62; 32]).unwrap();
        let append_unsigned_fields = |stream: &mut RlpStream| {
            stream.append(&H256::zero());
            stream.append(&H256::from_low_u64_be(12));
            stream.append(&H256::zero());
            stream.append(&H256::from_low_u64_be(13));
            stream.append(&2_u64);
            stream.append(&8_u64);
            stream.begin_list(1);
            stream.append(&reward_vote_hash);
        };
        let mut unsigned = RlpStream::new_list(7);
        append_unsigned_fields(&mut unsigned);
        let mut digest = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&unsigned.out());
        hasher.finalize(&mut digest);
        let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&digest).unwrap();
        let mut signature = signature.to_bytes().to_vec();
        signature.push(recovery_id.to_byte());

        let mut block = RlpStream::new_list(8);
        append_unsigned_fields(&mut block);
        block.append(&signature);
        let previous_bundle = optimized_cert_bundle(previous_votes);
        let mut period_data = RlpStream::new_list(4);
        period_data.append_raw(&block.out(), 1);
        period_data.append_raw(&previous_bundle, 1);
        period_data.append_empty_data();
        period_data.begin_list(0);
        let period_data_rlp = period_data.out().to_vec();
        let mut packet = RlpStream::new_list(3);
        packet.append(&true);
        packet.append_raw(&period_data_rlp, 1);
        packet.append_empty_data();
        (packet.out().to_vec(), period_data_rlp)
    }

    #[test]
    fn sync_ingress_successfully_enqueues_exact_period_child_and_peer() {
        let (path, storage) = temp_storage("pbft_sync_ingress_exact_enqueue");
        let service = PbftService::restore(storage.clone(), config(10)).unwrap();
        let final_chain = final_chain_with_pillar_voters(storage, &[]);
        let (packet_rlp, period_data_rlp) = signed_period_one_sync_packet();
        let peer = [0x5a; 64];

        let step = service
            .begin_pbft_sync_ingress(&final_chain, &packet_rlp, 41, peer, Vec::new())
            .unwrap();
        assert_eq!(step.action, PbftSyncIngressAction::EnqueuedContinue);
        assert_eq!(step.source_payload_id, 41);
        let popped = service.pop_period_data_queue().unwrap();
        assert_eq!(popped.period_data_rlp, period_data_rlp);
        assert_eq!(popped.source_peer_id, peer);
        assert!(popped.cert_vote_rlps.is_empty());

        drop(final_chain);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn malformed_begin_replaces_and_clears_a_stale_ingress_session() {
        let (path, storage) = temp_storage("pbft_sync_ingress_stale_replacement");
        let service = PbftService::restore(storage.clone(), config(10)).unwrap();
        let final_chain = final_chain_with_pillar_voters(storage, &[]);
        let (packet_rlp, _) = signed_period_one_sync_packet();
        let packet = decode_pbft_sync_packet_precheck(&packet_rlp).unwrap();
        *service.sync_ingress.lock().unwrap() = Some(PbftSyncIngressSession {
            packet,
            source_payload_id: 1,
            source_peer_id: [1; 64],
            next_vote: 0,
            slashing_submitters: Vec::new(),
            pending_slashing: None,
        });

        let step = service
            .begin_pbft_sync_ingress(&final_chain, &[0xc1, 0x80], 42, [2; 64], Vec::new())
            .unwrap();
        assert_eq!(step.action, PbftSyncIngressAction::Malicious);
        assert_eq!(step.source_payload_id, 42);
        assert!(service.sync_ingress.lock().unwrap().is_none());

        drop(final_chain);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn weighted_sync_ingress_pauses_for_slashing_and_accepts_both_executor_reports() {
        for transaction_inserted in [false, true] {
            let (path, storage) = temp_storage(if transaction_inserted {
                "pbft_sync_ingress_slashing_inserted"
            } else {
                "pbft_sync_ingress_slashing_rejected"
            });
            let voter = voter_from_secret(&NODE_SECRET);
            let submitter = [0x77; 20];
            let service_config = config(10);
            let service = PbftService::restore(storage.clone(), service_config).unwrap();
            let final_chain = final_chain_with_vote_validator_and_account(
                storage.clone(),
                voter,
                vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
                5_000,
                submitter,
            );
            let previous_hash = H256::repeat_byte(0x31);
            service
                .pbft_chain_update(previous_hash, H256::zero())
                .unwrap();

            let conflicting = generated_vote_at_period(H256::repeat_byte(0x32), 1);
            let initial = service
                .admit_and_persist_verified_vote_with_final_chain(
                    &final_chain,
                    &conflicting.vote_rlp,
                    PbftVoteAdmissionValidationRequest {
                        strict_vrf: true,
                        committee_size: 1,
                        number_of_proposers: 1,
                        has_preverified_weight: false,
                        preverified_weight: 0,
                    },
                    PbftVoteEventFactFlags {
                        vote_already_known: false,
                        carries_proposed_block: false,
                        valid_stale_reward_vote: true,
                    },
                    PbftVoteProgressContext {
                        current_period: 1,
                        current_round: 1,
                        max_future_period_delta: 0,
                        two_t_plus_one_threshold: None,
                        require_proposed_block_sidecar: false,
                        slashing_enabled: true,
                    },
                    &[],
                )
                .unwrap();
            assert!(initial.validation.accepted);

            let incoming = generated_vote_at_period(previous_hash, 1);
            let (packet_rlp, _) =
                signed_period_two_sync_packet(&[incoming.vote_rlp.clone()], conflicting.vote_hash);
            let peer = [0x6b; 64];
            let awaiting = service
                .begin_pbft_sync_ingress(
                    &final_chain,
                    &packet_rlp,
                    81,
                    peer,
                    vec![SlashingSubmitterIdentity {
                        wallet_index: 0,
                        address: [0; 20],
                        nonce: U256::zero(),
                        balance: U256::from(1_000_000u64),
                    }],
                )
                .unwrap();
            assert_eq!(awaiting.action, PbftSyncIngressAction::AwaitingSlashing);
            let effect = awaiting
                .slashing_transaction_effect
                .expect("conflict emits a slashing transaction");
            let resumed = service
                .report_pbft_sync_ingress_slashing(
                    &final_chain,
                    effect.proof_hash,
                    transaction_inserted,
                )
                .unwrap();
            assert_eq!(resumed.action, PbftSyncIngressAction::Malicious);
            assert_eq!(
                resumed.error_code,
                "PBFT_REWARD_VOTES_MISSING_PREFERRED_ROUND"
            );
            assert!(service.sync_ingress.lock().unwrap().is_none());

            drop(final_chain);
            drop(service);
            let _ = fs::remove_dir_all(path);
        }
    }

    fn pbft_block_rlp(period: u64, timestamp: u64) -> (Vec<u8>, PbftBlockLink) {
        let mut stream = rlp::RlpStream::new_list(8);
        stream.append(&H256::from_low_u64_be(period));
        stream.append(&H256::from_low_u64_be(period + 1));
        stream.append(&H256::from_low_u64_be(period + 2));
        stream.append(&H256::from_low_u64_be(period + 3));
        stream.append(&period);
        stream.append(&timestamp);
        stream.append(&H256::from_low_u64_be(period + 4));
        stream.append(&vec![0u8; 65]);
        let rlp = stream.out().to_vec();
        let link =
            PbftBlockLink::try_from(SignedPbftBlockRlp::new(&rlp)).expect("decode should succeed");
        (rlp, link)
    }

    fn pbft_block_rlp_with_pivot(
        previous: H256,
        pivot: H256,
        period: u64,
    ) -> (Vec<u8>, PbftBlockLink) {
        let mut stream = RlpStream::new_list(8);
        stream.append(&previous);
        stream.append(&pivot);
        stream.append(&H256::zero());
        stream.append(&H256::zero());
        stream.append(&period);
        stream.append(&0_u64);
        stream.append(&H256::zero());
        stream.append(&vec![0_u8; 65]);
        let rlp = stream.out().to_vec();
        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&rlp))
            .expect("PBFT block should decode");
        (rlp, link)
    }

    fn proposed_admission_block_rlp(
        period: u64,
        pivot_hash: H256,
        order_hash: H256,
    ) -> (Vec<u8>, PbftBlockLink) {
        proposed_admission_block_rlp_with_shape(period, pivot_hash, order_hash, false, true)
    }

    fn proposed_admission_block_rlp_with_shape(
        period: u64,
        pivot_hash: H256,
        order_hash: H256,
        invalid_timestamp: bool,
        valid_signature: bool,
    ) -> (Vec<u8>, PbftBlockLink) {
        let append_unsigned_fields = |stream: &mut RlpStream| {
            stream.append(&H256::zero());
            stream.append(&pivot_hash);
            stream.append(&order_hash);
            stream.append(&H256::zero());
            stream.append(&period);
            if invalid_timestamp {
                stream.append(&H256::repeat_byte(0x55));
            } else {
                stream.append(&0_u64);
            }
            stream.begin_list(0);
        };
        let mut unsigned = RlpStream::new_list(7);
        append_unsigned_fields(&mut unsigned);
        let signature = if valid_signature {
            let signing_key = SigningKey::from_slice(&[0x63; 32]).unwrap();
            let mut digest = [0_u8; 32];
            let mut hasher = Keccak::v256();
            hasher.update(&unsigned.out());
            hasher.finalize(&mut digest);
            let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&digest).unwrap();
            let mut bytes = signature.to_bytes().to_vec();
            bytes.push(recovery_id.to_byte());
            bytes
        } else {
            vec![0_u8; 65]
        };
        let mut stream = RlpStream::new_list(8);
        append_unsigned_fields(&mut stream);
        stream.append(&signature);
        let rlp = stream.out().to_vec();
        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&rlp))
            .expect("proposed PBFT block should decode");
        (rlp, link)
    }

    fn period_data_with_pbft_block(block_rlp: &[u8]) -> Vec<u8> {
        let mut period_data = RlpStream::new_list(4);
        period_data.append_raw(block_rlp, 1);
        period_data.begin_list(0);
        period_data.begin_list(0);
        period_data.begin_list(0);
        period_data.out().to_vec()
    }

    fn candidate_dag_block_rlp(pivot: H256, gas_estimation: u64) -> Vec<u8> {
        candidate_dag_block_rlp_at_level(pivot, 1, gas_estimation)
    }

    fn candidate_dag_block_rlp_at_level(pivot: H256, level: u64, gas_estimation: u64) -> Vec<u8> {
        let mut vdf = RlpStream::new_list(4);
        vdf.append(&vec![0x11_u8; 80]);
        vdf.append(&vec![0x22_u8]);
        vdf.append(&vec![0x33_u8]);
        vdf.append(&1_u16);
        let mut block = RlpStream::new_list(8);
        block.append(&pivot);
        block.append(&level);
        block.append(&0_u64);
        block.append(&vdf.out().to_vec());
        block.begin_list(0);
        block.begin_list(0);
        block.append(&&[0_u8; 65][..]);
        block.append(&gas_estimation);
        block.out().to_vec()
    }

    fn native_proposal_fact(
        period: u64,
        genesis: H256,
        ghost_path: Vec<H256>,
        pbft_gas_limit: u64,
    ) -> PbftManagerProposalInitialFact {
        PbftManagerProposalInitialFact {
            period,
            round: 1,
            previous_pbft_block_hash: H256::repeat_byte(0x81),
            last_period_dag_anchor_hash: genesis,
            dag_genesis_hash: genesis,
            dag_blocks_size: 10,
            ghost_path_move_back: 0,
            pbft_gas_limit,
            extra_data_required: false,
            extra_data_available: true,
            final_chain_hash_valid: true,
            final_chain_hash: H256::repeat_byte(0x82),
            wallets: vec![crate::pbft_manager::PbftManagerProposalWalletFact {
                wallet_index: 7,
                dpos_eligible: true,
                sortition_valid: true,
            }],
            ghost_path,
            has_non_finalized_fallback: false,
            non_finalized_fallback_hash: H256::zero(),
        }
    }

    fn sync_pillar_admission_fact(
        block_period: u64,
    ) -> crate::pbft_sync::PbftSyncAdmissionInitialFact {
        crate::pbft_sync::PbftSyncAdmissionInitialFact {
            block_period,
            block_prev_hash: H256::repeat_byte(0xa1),
            chain_last_hash: H256::repeat_byte(0xa1),
            chain_last_period: block_period.saturating_sub(1),
            block_in_chain: false,
            candidate_final_chain_hash: H256::zero(),
            reward_vote_hashes: Vec::new(),
            dag_transaction_hashes: Vec::new(),
            period_data_transaction_hashes: Vec::new(),
            extra_data_required: true,
            extra_data_present: true,
            extra_data_pillar_block_hash_present: true,
            pillar_votes_required: true,
            pillar_votes_present: true,
            previous_cert_votes_present: true,
            previous_cert_first_vote_has_weight: false,
        }
    }

    fn advance_sync_admission_to_pillar(
        service: &PbftService,
        block_period: u64,
    ) -> crate::pbft_sync::PbftSyncAdmissionSessionStep {
        assert!(service.begin_pbft_sync_admission(sync_pillar_admission_fact(block_period)));
        let mut step = service.pbft_sync_admission_next().expect("sync admission");
        loop {
            if step.next_check
                == crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::ValidatePillarVotes
            {
                return step;
            }
            step = match step.next_check {
                crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::ValidateFinalChainHash => {
                    service
                        .report_pbft_sync_admission_status(
                            step.cursor,
                            step.next_check,
                            crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus::Valid,
                            crate::pbft_sync::PbftSyncFactStatus::Valid,
                        )
                        .expect("final chain report")
                }
                crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::CheckRewardVotes
                | crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::ValidateCertVotes => service
                    .report_pbft_sync_admission_status(
                        step.cursor,
                        step.next_check,
                        crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus::NotChecked,
                        crate::pbft_sync::PbftSyncFactStatus::Valid,
                    )
                    .expect("vote report"),
                crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::CheckTransactions => {
                    report_sync_admission_transactions_for_test(
                        service,
                        crate::pbft_sync::PbftSyncAdmissionTransactionReport {
                            missing_transaction_hashes: Vec::new(),
                            finalized_transaction_hashes: Vec::new(),
                            contains_finalized_transactions: false,
                        },
                    )
                }
                other => panic!("unexpected sync check {other:?}"),
            };
        }
    }

    fn sync_transaction_admission_fact(
        dag_transaction_hashes: Vec<H256>,
        period_data_transaction_hashes: Vec<H256>,
    ) -> crate::pbft_sync::PbftSyncAdmissionInitialFact {
        crate::pbft_sync::PbftSyncAdmissionInitialFact {
            block_period: 1,
            block_prev_hash: H256::repeat_byte(0xa1),
            chain_last_hash: H256::repeat_byte(0xa1),
            chain_last_period: 0,
            block_in_chain: false,
            candidate_final_chain_hash: H256::zero(),
            reward_vote_hashes: Vec::new(),
            dag_transaction_hashes,
            period_data_transaction_hashes,
            extra_data_required: false,
            extra_data_present: false,
            extra_data_pillar_block_hash_present: false,
            pillar_votes_required: false,
            pillar_votes_present: false,
            previous_cert_votes_present: true,
            previous_cert_first_vote_has_weight: false,
        }
    }

    fn report_sync_admission_transactions_for_test(
        service: &PbftService,
        report: crate::pbft_sync::PbftSyncAdmissionTransactionReport,
    ) -> crate::pbft_sync::PbftSyncAdmissionSessionStep {
        let identity = service
            .manager
            .pbft_sync_admission_transaction_request()
            .expect("transaction request identity");
        service
            .manager
            .report_pbft_sync_admission_transactions_exact(identity, report)
            .expect("exact transaction report")
    }

    fn advance_sync_admission_to_transactions(
        service: &PbftService,
        fact: crate::pbft_sync::PbftSyncAdmissionInitialFact,
    ) -> crate::pbft_sync::PbftSyncAdmissionSessionStep {
        assert!(service.begin_pbft_sync_admission(fact));
        let mut step = service.pbft_sync_admission_next().expect("sync admission");
        loop {
            if step.next_check
                == crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::CheckTransactions
            {
                return step;
            }
            step = match step.next_check {
                crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::ValidateFinalChainHash => {
                    service
                        .report_pbft_sync_admission_status(
                            step.cursor,
                            step.next_check,
                            crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus::Valid,
                            crate::pbft_sync::PbftSyncFactStatus::Valid,
                        )
                        .expect("final-chain report")
                }
                crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::CheckRewardVotes
                | crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::ValidateCertVotes => service
                    .report_pbft_sync_admission_status(
                        step.cursor,
                        step.next_check,
                        crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus::NotChecked,
                        crate::pbft_sync::PbftSyncFactStatus::Valid,
                    )
                    .expect("vote report"),
                other => panic!("unexpected sync check {other:?}"),
            };
        }
    }

    fn advance_sync_admission_to_reward_votes(
        service: &PbftService,
        reward_vote_hashes: Vec<H256>,
    ) -> crate::pbft_sync::PbftSyncAdmissionSessionStep {
        let mut fact = sync_transaction_admission_fact(Vec::new(), Vec::new());
        fact.block_period = 2;
        fact.chain_last_period = 1;
        fact.reward_vote_hashes = reward_vote_hashes;
        assert!(service.begin_pbft_sync_admission(fact));
        let final_chain = service.pbft_sync_admission_next().expect("sync admission");
        let reward = service
            .report_pbft_sync_admission_status(
                final_chain.cursor,
                final_chain.next_check,
                crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus::Valid,
                crate::pbft_sync::PbftSyncFactStatus::Valid,
            )
            .expect("final-chain report");
        assert_eq!(
            reward.next_check,
            crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::CheckRewardVotes
        );
        reward
    }

    fn dynamic_lambda_fact(finalized_period: u64) -> PbftDynamicLambdaFact {
        PbftDynamicLambdaFact {
            dynamic_lambda_active: true,
            finalized_period,
            finalized_round: 1,
            pre_adjust_rounds_count_dynamic_lambda: 9,
            pre_adjust_dynamic_lambda: 1_500,
            config: crate::pbft_finalize::PbftDynamicLambdaConfig {
                cacti_block_num: 10,
                lambda_min: 500,
                lambda_max: 1_500,
                lambda_default: 2_000,
                lambda_change_interval: 10,
                lambda_change: 10,
                consensus_delay: 400,
                dpos_blocks_per_year: 500,
            },
        }
    }

    fn dag_service(storage: Arc<Storage>) -> DagTransactionService {
        DagTransactionService::restore(
            storage,
            DagTransactionServiceConfig {
                transaction: TransactionServiceConfig {
                    queue_max_size: 16,
                    gas_pricer_config: GasPricerConfig {
                        percentile: 50,
                        minimum_price: U256::one(),
                        history_blocks: 0,
                        is_light_node: false,
                        blocks_gas_pricer: false,
                    },
                    proposal_dag_gas_limit: 1_000_000,
                },
                dag: DagServiceConfig {
                    genesis_hash: H256::repeat_byte(1),
                    dag_expiry_limit: 32,
                    max_levels_per_period: 100,
                },
                sortition: SortitionConfig {
                    params: SortitionParams {
                        vrf: VrfParams {
                            threshold_upper: 0x100,
                        },
                        vdf: VdfParams {
                            difficulty_min: 1,
                            difficulty_max: 10,
                            difficulty_stale: 5,
                            lambda_bound: 100,
                        },
                    },
                    changes_count_for_average: 8,
                    dag_efficiency_targets: (5_000, 10_000),
                    changing_interval: 10,
                    computation_interval: 5,
                },
            },
        )
        .expect("DAG transaction service restores")
    }

    fn voter_from_secret(secret: &[u8; 32]) -> [u8; 20] {
        let key = SigningKey::from_slice(secret).unwrap();
        let public_key = key.verifying_key().to_encoded_point(false);
        let mut output = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&public_key.as_bytes()[1..]);
        hasher.finalize(&mut output);
        output[12..].try_into().unwrap()
    }

    fn signed_pillar_vote(
        secret: [u8; 32],
        period: u64,
        block_hash: H256,
    ) -> (PillarVote, [u8; 20]) {
        let key = SigningKey::from_slice(&secret).unwrap();
        let mut vote = PillarVote {
            period,
            block_hash,
            signature: [0; 65],
        };
        let (signature, recovery_id) = key
            .sign_prehash_recoverable(vote.hash(false).as_bytes())
            .unwrap();
        vote.signature[..64].copy_from_slice(&signature.to_bytes());
        vote.signature[64] = recovery_id.to_byte();
        (vote, voter_from_secret(&secret))
    }

    fn pillar_current_data(period: u64) -> (PillarBlock, Vec<u8>) {
        let block = PillarBlock {
            period,
            state_root: H256::from_low_u64_be(1),
            previous_pillar_block_hash: H256::from_low_u64_be(2),
            bridge_root: H256::from_low_u64_be(3),
            epoch: 4,
            validator_vote_count_changes: Vec::new(),
        };
        let rlp = CurrentPillarBlockDataDb {
            pillar_block: block.clone(),
            vote_counts: Vec::new(),
        }
        .encode_rlp();
        (block, rlp)
    }

    fn final_chain_with_pillar_voters(storage: Arc<Storage>, voters: &[[u8; 20]]) -> FinalChain {
        final_chain_with_pillar_voters_and_delay(storage, voters, 0)
    }

    fn final_chain_with_pillar_voters_and_delay(
        storage: Arc<Storage>,
        voters: &[[u8; 20]],
        delegation_delay: u64,
    ) -> FinalChain {
        use rustaxa_types::{
            DposTokenAmount, GenesisDposConfig, GenesisValidator, GenesisValidatorMetadata,
        };

        let stake = U256::from(5_000u64).to_big_endian().to_vec();
        FinalChain::new(
            storage,
            0.into(),
            0,
            Vec::new(),
            voters
                .iter()
                .map(|address| GenesisValidator {
                    address: *address,
                    vrf_key: [address[0]; 32],
                    total_stake: stake.clone(),
                    delegations: vec![(*address, stake.clone())],
                    metadata: GenesisValidatorMetadata {
                        owner: *address,
                        commission: 0,
                        description: String::new(),
                        endpoint: String::new(),
                    },
                })
                .collect(),
            GenesisDposConfig {
                eligibility_balance_threshold: DposTokenAmount::from(U256::from(1_000u64)),
                vote_eligibility_balance_step: DposTokenAmount::from(U256::from(1_000u64)),
                validator_maximum_stake: DposTokenAmount::from(U256::from(30_000u64)),
                minimum_deposit: DposTokenAmount::zero(),
                commission_change_delta: 0,
                commission_change_frequency: 0,
                delegation_delay,
                dag_vdf_sortition_total_vote_count_until_period: 0.into(),
            },
        )
        .unwrap()
    }

    fn final_chain_with_vote_validator(
        storage: Arc<Storage>,
        voter: [u8; 20],
        vrf_key: [u8; 32],
        stake: u64,
    ) -> FinalChain {
        final_chain_with_vote_validator_and_delay(storage, voter, vrf_key, stake, 0)
    }

    fn final_chain_with_vote_validator_and_delay(
        storage: Arc<Storage>,
        voter: [u8; 20],
        vrf_key: [u8; 32],
        stake: u64,
        delegation_delay: u64,
    ) -> FinalChain {
        use rustaxa_types::{
            DposTokenAmount, GenesisDposConfig, GenesisValidator, GenesisValidatorMetadata,
        };

        let stake = U256::from(stake).to_big_endian().to_vec();
        FinalChain::new(
            storage,
            0.into(),
            0,
            Vec::new(),
            vec![GenesisValidator {
                address: voter,
                vrf_key,
                total_stake: stake.clone(),
                delegations: vec![(voter, stake)],
                metadata: GenesisValidatorMetadata {
                    owner: voter,
                    commission: 0,
                    description: String::new(),
                    endpoint: String::new(),
                },
            }],
            GenesisDposConfig {
                eligibility_balance_threshold: DposTokenAmount::from(U256::from(1_000u64)),
                vote_eligibility_balance_step: DposTokenAmount::from(U256::from(1_000u64)),
                validator_maximum_stake: DposTokenAmount::from(U256::from(30_000u64)),
                minimum_deposit: DposTokenAmount::zero(),
                commission_change_delta: 0,
                commission_change_frequency: 0,
                delegation_delay,
                dag_vdf_sortition_total_vote_count_until_period: 0.into(),
            },
        )
        .unwrap()
    }

    fn final_chain_with_vote_validator_and_account(
        storage: Arc<Storage>,
        voter: [u8; 20],
        vrf_key: [u8; 32],
        stake: u64,
        account: [u8; 20],
    ) -> FinalChain {
        use rustaxa_types::{
            DposTokenAmount, FinalChainAccountBalance, GenesisAccount, GenesisDposConfig,
            GenesisValidator, GenesisValidatorMetadata,
        };

        let stake = U256::from(stake).to_big_endian().to_vec();
        FinalChain::new(
            storage,
            0.into(),
            0,
            vec![GenesisAccount {
                address: account,
                balance: FinalChainAccountBalance::from_cpp_genesis_bytes(
                    &U256::from(1_000_000u64).to_big_endian(),
                )
                .unwrap(),
            }],
            vec![GenesisValidator {
                address: voter,
                vrf_key,
                total_stake: stake.clone(),
                delegations: vec![(voter, stake)],
                metadata: GenesisValidatorMetadata {
                    owner: voter,
                    commission: 0,
                    description: String::new(),
                    endpoint: String::new(),
                },
            }],
            GenesisDposConfig {
                eligibility_balance_threshold: DposTokenAmount::from(U256::from(1_000u64)),
                vote_eligibility_balance_step: DposTokenAmount::from(U256::from(1_000u64)),
                validator_maximum_stake: DposTokenAmount::from(U256::from(30_000u64)),
                minimum_deposit: DposTokenAmount::zero(),
                commission_change_delta: 0,
                commission_change_frequency: 0,
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 0.into(),
            },
        )
        .unwrap()
    }

    fn generated_vote_at_period(
        block_hash: H256,
        period: u64,
    ) -> crate::pbft_vote_generation::PbftGeneratedVote {
        generate_pbft_vote(PbftVoteGenerationInput {
            block_hash,
            vote_type: PbftVoteType::Cert,
            period,
            round: 2,
            step: 3,
            node_secret: NODE_SECRET,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&NODE_SECRET).into(),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap()
    }

    fn vote_validation_request(
        has_preverified_weight: bool,
        preverified_weight: u64,
    ) -> PbftVoteAdmissionValidationRequest {
        PbftVoteAdmissionValidationRequest {
            strict_vrf: true,
            committee_size: 100,
            number_of_proposers: 20,
            has_preverified_weight,
            preverified_weight,
        }
    }

    fn invalid_signature_vote(block_hash: H256, period: u64) -> Vec<u8> {
        let vote = generated_vote_at_period(block_hash, period);
        let vote_rlp = Rlp::new(&vote.vote_rlp);
        let mut invalid_signature_vote = RlpStream::new_list(3);
        let block_hash: H256 = vote_rlp.val_at(0).unwrap();
        let sortition_rlp: Vec<u8> = vote_rlp.val_at(1).unwrap();
        invalid_signature_vote.append(&block_hash);
        invalid_signature_vote.append(&sortition_rlp);
        invalid_signature_vote.append(&vec![0_u8; 65]);
        invalid_signature_vote.out().to_vec()
    }

    fn service_with_test_chain(stake: u64, voter_secret: [u8; 32]) -> (PbftService, FinalChain) {
        let (_, storage) = temp_storage("pbft_service_vote_generation_chain");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        let voter = voter_from_secret(&voter_secret);
        let final_chain = final_chain_with_vote_validator(
            storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            stake,
        );
        (service, final_chain)
    }

    fn vote_input(
        vote_type: PbftVoteType,
        step: u64,
        secret: [u8; 32],
        period: u64,
    ) -> PbftVoteGenerationInput {
        PbftVoteGenerationInput {
            block_hash: H256::from_low_u64_be(0x11),
            vote_type,
            period,
            round: 1,
            step,
            node_secret: secret,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&secret).into(),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        }
    }

    fn proposer_sortition_request(
        voter_secret: [u8; 32],
        pbft_period: u64,
        pbft_round: u64,
        number_of_proposers: u64,
    ) -> PbftProposerSortitionRequest {
        let expected_voter = voter_from_secret(&voter_secret);
        let signing_key = SigningKey::from_slice(&voter_secret).unwrap();
        let point = signing_key.verifying_key().to_encoded_point(false);
        let mut voter_public_key = [0_u8; 64];
        voter_public_key.copy_from_slice(&point.as_bytes()[1..]);

        PbftProposerSortitionRequest {
            pbft_period,
            pbft_round,
            number_of_proposers,
            vrf_secret: VRF_SECRET,
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            voter_public_key,
            expected_voter: H160::from(expected_voter),
        }
    }

    fn vote_progress_context(threshold: u64) -> PbftVoteProgressContext {
        PbftVoteProgressContext {
            current_period: 12,
            current_round: 2,
            max_future_period_delta: 0,
            two_t_plus_one_threshold: Some(threshold),
            require_proposed_block_sidecar: false,
            slashing_enabled: true,
        }
    }

    fn vote_event_flags() -> PbftVoteEventFactFlags {
        PbftVoteEventFactFlags {
            vote_already_known: false,
            carries_proposed_block: true,
            valid_stale_reward_vote: false,
        }
    }

    #[test]
    fn composed_vote_admission_accepts_preverified_weight() {
        let (_path, storage) = temp_storage("pbft_service_vote_admission_preverified");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let final_chain = final_chain_with_vote_validator(
            storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            5_000,
        );
        let vote = generated_vote_at_period(H256::repeat_byte(0x73), 12);

        let result = service
            .admit_and_persist_verified_vote_with_final_chain(
                &final_chain,
                &vote.vote_rlp,
                vote_validation_request(true, 40),
                vote_event_flags(),
                vote_progress_context(80),
                &[],
            )
            .unwrap();

        assert!(result.validation.accepted);
        assert_eq!(result.validation.calculated_weight, 40);
        assert!(result.transaction.transition_published);
        assert!(!result.transaction.persistence_required);
        assert_eq!(
            result
                .transaction
                .outcome
                .precheck
                .progress_fact
                .expect("accepted vote fact")
                .weight,
            40
        );
    }

    #[test]
    fn ordinary_vote_admission_does_not_resolve_slashing_accounts() {
        use std::cell::Cell;

        let (_path, storage) = temp_storage("pbft_service_vote_admission_lazy_slashing");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let final_chain = final_chain_with_vote_validator(
            storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            5_000,
        );
        let vote = generated_vote_at_period(H256::repeat_byte(0x75), 12);
        let request = vote_validation_request(true, 40);
        let (validation, _) = service
            .validate_verified_vote_with_final_chain_internal(
                &final_chain,
                &vote.vote_rlp,
                request,
                false,
            )
            .unwrap();
        let resolver_called = Cell::new(false);

        let result = service
            .admit_validated_vote_with_slashing_resolver(
                &vote.vote_rlp,
                &validation,
                vote_event_flags(),
                vote_progress_context(80),
                None,
                || {
                    resolver_called.set(true);
                    anyhow::bail!("FINAL_CHAIN_ACCOUNT_SNAPSHOT_UNAVAILABLE")
                },
            )
            .unwrap();

        assert!(result.validation.accepted);
        assert!(result.transaction.transition_published);
        assert!(result.slashing_transaction_effect.is_none());
        assert!(!resolver_called.get());
    }

    #[test]
    fn network_root_admits_an_ordinary_vote_without_slashing_account_facts() {
        let (_path, storage) = temp_storage("pbft_service_network_vote_admission");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let final_chain = final_chain_with_vote_validator(
            storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            5_000,
        );
        let vote = generated_vote_at_period(H256::repeat_byte(0x76), 1);
        let unavailable_submitter = SlashingSubmitterIdentity {
            wallet_index: 0,
            address: [0x99; 20],
            nonce: U256::zero(),
            balance: U256::zero(),
        };

        let result = service
            .admit_network_verified_vote(&final_chain, &vote.vote_rlp, &[unavailable_submitter])
            .unwrap();

        assert!(result.validation.accepted);
        assert!(result.transaction.transition_published);
        assert!(result.slashing_transaction_effect.is_none());
    }

    #[test]
    fn composed_vote_admission_rejects_zero_stake() {
        let (_path, storage) = temp_storage("pbft_service_vote_admission_zero_stake");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let final_chain = final_chain_with_vote_validator(
            storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            500,
        );
        let vote = generated_vote_at_period(H256::repeat_byte(0x74), 1);

        let result = service
            .admit_and_persist_verified_vote_with_final_chain(
                &final_chain,
                &vote.vote_rlp,
                vote_validation_request(false, 0),
                vote_event_flags(),
                vote_progress_context(80),
                &[],
            )
            .unwrap();

        assert_eq!(
            result.validation.status,
            PbftVoteValidationStatus::ZeroStake
        );
        assert!(result.transaction.transition_published);
        assert!(!result.transaction.persistence_required);
        assert!(result.transaction.outcome.add_outcome.is_none());
        assert!(
            service
                .verified_votes()
                .lock()
                .unwrap()
                .replay_contains(vote.vote_hash)
        );
    }

    #[test]
    fn composed_vote_validation_marks_replay_once() {
        let (_path, storage) = temp_storage("pbft_service_vote_validation_replay");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let final_chain = final_chain_with_vote_validator(
            storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            5_000,
        );
        let vote = generated_vote_at_period(H256::repeat_byte(0x71), 1);

        let (first, first_replay, weighted_vote_rlp) = service
            .validate_verified_vote_with_final_chain(
                &final_chain,
                &vote.vote_rlp,
                vote_validation_request(false, 0),
            )
            .unwrap();
        assert_eq!(first.status, PbftVoteValidationStatus::Valid);
        assert!(first.accepted && first.weight_calculated && first.calculated_weight > 0);
        assert_eq!(
            weighted_vote_rlp,
            Some(
                build_weighted_pbft_vote_payload(&vote.vote_rlp, first.calculated_weight)
                    .unwrap()
                    .vote_rlp
            )
        );
        assert!(first_replay.should_mark);
        assert!(first_replay.inserted);
        assert!(!first_replay.already_present);

        let (_, repeated_replay, _) = service
            .validate_verified_vote_with_final_chain(
                &final_chain,
                &vote.vote_rlp,
                vote_validation_request(false, 0),
            )
            .unwrap();
        assert!(!repeated_replay.inserted);
        assert!(repeated_replay.already_present);
    }

    #[test]
    fn composed_vote_validation_malformed_canonical_rlp_no_replay_mark() {
        let (_path, storage) = temp_storage("pbft_service_vote_validation_malformed_no_replay");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let final_chain = final_chain_with_vote_validator(
            storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            5_000,
        );

        let malformed = vec![0x01, 0x02, 0x03];
        let (first, first_replay, weighted_vote_rlp) = service
            .validate_verified_vote_with_final_chain(
                &final_chain,
                &malformed,
                vote_validation_request(false, 0),
            )
            .unwrap();
        assert_eq!(first.status, PbftVoteValidationStatus::InvalidVoteType);
        assert_eq!(first.error_code, "PBFT_CANONICAL_VOTE_MALFORMED_RLP");
        assert!(!first.mark_validated_replay);
        assert!(!first_replay.should_mark);
        assert!(!first_replay.inserted);
        assert!(!first_replay.already_present);
        assert!(weighted_vote_rlp.is_none());

        let (_, second_replay, _) = service
            .validate_verified_vote_with_final_chain(
                &final_chain,
                &malformed,
                vote_validation_request(false, 0),
            )
            .unwrap();
        assert!(!second_replay.should_mark);
        assert!(!second_replay.inserted);
        assert!(!second_replay.already_present);
    }

    #[test]
    fn composed_vote_validation_invalid_signature_replay_marked_once() {
        let (_path, storage) =
            temp_storage("pbft_service_vote_validation_invalid_signature_replay");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let final_chain = final_chain_with_vote_validator(
            storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            5_000,
        );

        let invalid_signature = invalid_signature_vote(H256::repeat_byte(0x73), 1);
        let (first, first_replay, weighted_vote_rlp) = service
            .validate_verified_vote_with_final_chain(
                &final_chain,
                &invalid_signature,
                vote_validation_request(false, 0),
            )
            .unwrap();
        assert_eq!(first.status, PbftVoteValidationStatus::InvalidSignature);
        assert_eq!(first.error_code, "PBFT_VOTE_VALIDATION_INVALID_SIGNATURE");
        assert!(first.mark_validated_replay);
        assert!(first_replay.should_mark);
        assert!(first_replay.inserted);
        assert!(!first_replay.already_present);
        assert!(weighted_vote_rlp.is_none());

        let (_, second_replay, _) = service
            .validate_verified_vote_with_final_chain(
                &final_chain,
                &invalid_signature,
                vote_validation_request(false, 0),
            )
            .unwrap();
        assert!(second_replay.should_mark);
        assert!(!second_replay.inserted);
        assert!(second_replay.already_present);
    }

    #[test]
    fn composed_vote_validation_stops_after_zero_voter_stake() {
        let (_path, storage) = temp_storage("pbft_service_vote_validation_zero_stake");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let final_chain = final_chain_with_vote_validator(
            storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            500,
        );
        let vote = generated_vote_at_period(H256::repeat_byte(0x72), 1);

        let (validation, replay, weighted_vote_rlp) = service
            .validate_verified_vote_with_final_chain(
                &final_chain,
                &vote.vote_rlp,
                vote_validation_request(false, 0),
            )
            .unwrap();

        assert_eq!(validation.status, PbftVoteValidationStatus::ZeroStake);
        assert!(validation.rejected);
        assert!(!validation.vrf_valid);
        assert!(!validation.weight_calculated);
        assert!(weighted_vote_rlp.is_none());
        assert!(replay.inserted);
    }

    fn threshold_fact(period: u64) -> PbftTwoTPlusOneThresholdFact {
        PbftTwoTPlusOneThresholdFact {
            pbft_period: period,
            vote_type: PbftVoteType::Cert,
            current_pbft_chain_size: 0,
            committee_size: 100,
            number_of_proposers: 20,
            has_total_dpos_votes_count: false,
            total_dpos_votes_count: 0,
            future_dpos_state: false,
            unknown_error: false,
        }
    }

    #[test]
    fn composed_threshold_uses_native_chain_and_final_chain_state() {
        let (_path, storage) = temp_storage("pbft_service_threshold_ready");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let final_chain = final_chain_with_vote_validator(
            storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            5_000,
        );

        let mut request = threshold_fact(0);
        request.has_total_dpos_votes_count = true;
        request.total_dpos_votes_count = 999;
        request.future_dpos_state = true;
        request.unknown_error = true;
        let plan = service
            .verified_votes_two_t_plus_one_threshold_with_final_chain(&final_chain, request)
            .unwrap();

        assert_eq!(plan.status, PbftTwoTPlusOneThresholdStatus::Available);
        assert!(plan.has_threshold);
        assert_eq!(plan.threshold, 4);
    }

    #[test]
    fn composed_threshold_reports_future_final_chain_state() {
        let (_path, storage) = temp_storage("pbft_service_threshold_future");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let final_chain = final_chain_with_vote_validator(
            storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            5_000,
        );

        let plan = service
            .verified_votes_two_t_plus_one_threshold_with_final_chain(
                &final_chain,
                threshold_fact(1),
            )
            .unwrap();

        assert_eq!(plan.status, PbftTwoTPlusOneThresholdStatus::FutureDposState);
        assert!(!plan.has_threshold);
        assert_eq!(plan.error_code, "PBFT_TWO_T_PLUS_ONE_FUTURE_DPOS_STATE");
    }

    #[test]
    fn composed_threshold_cache_hit_skips_final_chain_state() {
        let (_path, storage) = temp_storage("pbft_service_threshold_cache");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let final_chain = final_chain_with_vote_validator(
            storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            5_000,
        );
        let mut seeded = threshold_fact(1);
        seeded.current_pbft_chain_size = 1;
        seeded.has_total_dpos_votes_count = true;
        seeded.total_dpos_votes_count = 100;
        let seeded_plan = service
            .verified_votes()
            .lock()
            .unwrap()
            .plan_two_t_plus_one_threshold(seeded);
        assert!(seeded_plan.cached);

        let cached = service
            .verified_votes_two_t_plus_one_threshold_with_final_chain(
                &final_chain,
                threshold_fact(1),
            )
            .unwrap();

        assert_eq!(cached.status, PbftTwoTPlusOneThresholdStatus::Available);
        assert_eq!(cached.threshold, seeded_plan.threshold);
        assert!(cached.cache_hit);
    }

    #[test]
    fn composed_service_generates_and_validates_local_proposer_sortition() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let request = proposer_sortition_request(NODE_SECRET, 1, 1, 100);

        let result = service
            .generate_and_validate_proposer_sortition(&final_chain, request)
            .unwrap();

        assert_eq!(result.status, PbftProposerSortitionStatus::Valid);
        assert!(result.accepted);
        assert_eq!(result.error_code, "");
    }

    #[test]
    fn composed_service_generates_and_validates_local_proposer_sortition_deterministically() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let request = proposer_sortition_request(NODE_SECRET, 1, 1, 100);

        let first = service
            .generate_and_validate_proposer_sortition(&final_chain, request)
            .unwrap();
        let second = service
            .generate_and_validate_proposer_sortition(
                &final_chain,
                proposer_sortition_request(NODE_SECRET, 1, 1, 100),
            )
            .unwrap();

        assert_eq!(first.status, second.status);
        assert_eq!(first.accepted, second.accepted);
        assert_eq!(first.error_code, second.error_code);
    }

    #[test]
    fn composed_service_reports_proposer_sortition_zero_stake_as_typed_status() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let request = proposer_sortition_request(NODE_SECRET_ZERO_STAKE, 1, 1, 100);

        let result = service
            .generate_and_validate_proposer_sortition(&final_chain, request)
            .unwrap();

        assert_eq!(result.status, PbftProposerSortitionStatus::ZeroStake);
        assert!(!result.accepted);
    }

    #[test]
    fn composed_service_reports_proposer_sortition_future_as_typed_status() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let request = proposer_sortition_request(NODE_SECRET, 99, 1, 100);

        let result = service
            .generate_and_validate_proposer_sortition(&final_chain, request)
            .unwrap();

        assert_eq!(result.status, PbftProposerSortitionStatus::FutureDposState);
    }

    #[test]
    fn composed_service_errors_on_proposer_sortition_period_zero() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let request = proposer_sortition_request(NODE_SECRET, 0, 1, 100);

        let err = service
            .generate_and_validate_proposer_sortition(&final_chain, request)
            .expect_err("period 0 must underflow");
        assert!(
            err.to_string()
                .contains("PBFT_PROPOSER_SORTITION_PERIOD_UNDERFLOW")
        );
    }

    #[test]
    fn composed_service_errors_on_proposer_sortition_identity_mismatch() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let mut request = proposer_sortition_request(NODE_SECRET, 1, 1, 100);
        request.expected_voter = H160::from([0xAA; 20]);

        let err = service
            .generate_and_validate_proposer_sortition(&final_chain, request)
            .expect_err("identity mismatch must be a boundary error");
        assert!(
            err.to_string()
                .contains("PBFT_PROPOSER_SORTITION_INVALID_VOTER_IDENTITY")
        );
    }

    #[test]
    fn composed_service_validates_proposer_sortition_identity_before_future_period_lookup() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let mut request = proposer_sortition_request(NODE_SECRET, 99, 1, 100);
        request.expected_voter = H160::from([0xAA; 20]);

        let err = service
            .generate_and_validate_proposer_sortition(&final_chain, request)
            .expect_err("identity mismatch must be checked before FinalChain state");
        assert!(
            err.to_string()
                .contains("PBFT_PROPOSER_SORTITION_INVALID_VOTER_IDENTITY")
        );
    }

    #[test]
    fn composed_service_validates_proposer_sortition_vrf_public_key_before_future_period_lookup() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let mut request = proposer_sortition_request(NODE_SECRET, 99, 1, 100);
        request.expected_vrf_public_key = [0_u8; 32];

        let err = service
            .generate_and_validate_proposer_sortition(&final_chain, request)
            .expect_err("vrf mismatch must be checked before FinalChain state");
        assert!(
            err.to_string()
                .contains("PBFT_PROPOSER_SORTITION_INVALID_VRF_PUBLIC_KEY")
        );
    }

    #[test]
    fn composed_service_errors_on_proposer_sortition_vrf_mismatch() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let mut request = proposer_sortition_request(NODE_SECRET, 1, 1, 100);
        request.expected_vrf_public_key = [0_u8; 32];

        let err = service
            .generate_and_validate_proposer_sortition(&final_chain, request)
            .expect_err("vrf public key mismatch must be a boundary error");
        assert!(
            err.to_string()
                .contains("PBFT_PROPOSER_SORTITION_INVALID_VRF_PUBLIC_KEY")
        );
    }

    #[test]
    fn composed_service_generates_weighted_vote_bytes() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let vote = service
            .generate_signed_vote_with_weight(
                &final_chain,
                vote_input(PbftVoteType::Propose, 1, NODE_SECRET, 1),
                50,
                100,
            )
            .unwrap();

        assert!(vote.accepted);
        assert!(vote.has_weight);
        assert!(vote.weight > 0);
    }

    #[test]
    fn composed_service_generates_weighted_zero_stake_vote() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let vote = service
            .generate_signed_vote_with_weight(
                &final_chain,
                vote_input(PbftVoteType::Propose, 1, NODE_SECRET_ZERO_STAKE, 1),
                50,
                100,
            )
            .unwrap();

        assert_eq!(vote.status, PbftVoteGenerationStatus::ZeroStake);
        assert!(!vote.accepted);
    }

    #[test]
    fn composed_service_reports_zero_weight_as_typed_generation_status() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let vote = service
            .generate_signed_vote_with_weight(
                &final_chain,
                vote_input(PbftVoteType::Propose, 1, NODE_SECRET, 1),
                0,
                0,
            )
            .unwrap();

        assert_eq!(vote.status, PbftVoteGenerationStatus::ZeroWeight);
        assert!(!vote.accepted);
    }

    #[test]
    fn composed_service_preserves_identity_error_before_zero_stake_status() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let mut mismatched = vote_input(PbftVoteType::Propose, 1, NODE_SECRET, 1);
        mismatched.expected_voter = H160::from([0xAA; 20]);

        let vote = service
            .generate_signed_vote_with_weight(&final_chain, mismatched, 50, 100)
            .unwrap();

        assert_eq!(vote.status, PbftVoteGenerationStatus::NodeSecretMismatch);
        assert!(!vote.accepted);
    }

    #[test]
    fn composed_service_generates_weighted_vote_from_future_period() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        assert!(
            service
                .generate_signed_vote_with_weight(
                    &final_chain,
                    vote_input(PbftVoteType::Propose, 1, NODE_SECRET, 99),
                    50,
                    100
                )
                .is_err()
        );
    }

    #[test]
    fn composed_service_generates_weighted_vote_from_period_zero() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        assert!(
            service
                .generate_signed_vote_with_weight(
                    &final_chain,
                    vote_input(PbftVoteType::Propose, 1, NODE_SECRET, 0),
                    50,
                    100
                )
                .is_err()
        );
    }

    #[test]
    fn composed_service_rejects_invalid_weighted_vote_type() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let invalid_input = vote_input(PbftVoteType::Invalid, 1, NODE_SECRET, 1);

        let vote = service
            .generate_signed_vote_with_weight(&final_chain, invalid_input, 50, 100)
            .unwrap();

        assert_eq!(vote.status, PbftVoteGenerationStatus::InvalidVoteType);
        assert!(!vote.accepted);
    }

    #[test]
    fn composed_service_collects_pbft_dpos_total_vote_count() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let ready = service
            .collect_dpos_total_vote_count(
                &final_chain,
                crate::pbft_vote_generation::PbftFinalChainDposTotalVoteCountRequest { period: 0 },
            )
            .expect("ready total-vote lookup should return data");

        assert_eq!(ready.status.as_u8(), 0);
        assert!(ready.status.is_ready());
        assert_eq!(ready.status.data_or_zero(), 5);
        assert_eq!(
            ready.last_block_number,
            rustaxa_types::FinalChainBlockNumber::GENESIS
        );

        let future = service
            .collect_dpos_total_vote_count(
                &final_chain,
                crate::pbft_vote_generation::PbftFinalChainDposTotalVoteCountRequest { period: 99 },
            )
            .expect("future total-vote lookup should be returned as unavailable data");
        assert_eq!(future.status.as_u8(), 1);
        assert!(future.status.is_unavailable());
        assert_eq!(
            future.status.error_code(),
            "PBFT_FINAL_CHAIN_TOTAL_VOTES_FUTURE_PERIOD"
        );
    }

    #[test]
    fn composed_service_collects_pbft_dpos_wallet_aggregate_vote_count() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let validator = voter_from_secret(&NODE_SECRET);
        let ready = service
            .collect_dpos_wallet_aggregate_vote_count(
                &final_chain,
                crate::pbft_vote_generation::PbftFinalChainDposWalletAggregateVoteCountRequest {
                    period: 0,
                    eligible_wallet_period: 0,
                    addresses: vec![H160::from(validator), H160::from([0xA1; 20])],
                },
            )
            .expect("ready aggregate vote lookup should return data");

        assert_eq!(ready.status.as_u8(), 0);
        assert!(ready.status.is_ready());
        assert!(ready.eligible_wallet_period_ready);
        assert_eq!(ready.status.data_or_zero(), 5);

        let duplicate = service
            .collect_dpos_wallet_aggregate_vote_count(
                &final_chain,
                crate::pbft_vote_generation::PbftFinalChainDposWalletAggregateVoteCountRequest {
                    period: 0,
                    eligible_wallet_period: 0,
                    addresses: vec![H160::from(validator), H160::from(validator)],
                },
            )
            .expect("duplicate wallets remain part of the aggregate");
        assert_eq!(duplicate.status.data_or_zero(), 10);
        assert!(duplicate.eligible_wallet_period_ready);

        let empty_future = service
            .collect_dpos_wallet_aggregate_vote_count(
                &final_chain,
                crate::pbft_vote_generation::PbftFinalChainDposWalletAggregateVoteCountRequest {
                    period: 99,
                    eligible_wallet_period: 99,
                    addresses: Vec::new(),
                },
            )
            .expect("empty aggregates do not require period state");
        assert!(empty_future.status.is_ready());
        assert_eq!(empty_future.status.data_or_zero(), 0);
        assert!(empty_future.eligible_wallet_period_ready);

        let ready_future = service
            .collect_dpos_wallet_aggregate_vote_count(
                &final_chain,
                crate::pbft_vote_generation::PbftFinalChainDposWalletAggregateVoteCountRequest {
                    period: 0,
                    eligible_wallet_period: 1,
                    addresses: vec![H160::from(validator)],
                },
            )
            .expect("period mismatch must short-circuit with deterministic unavailable data");
        assert_eq!(ready_future.status.as_u8(), 1);
        assert!(ready_future.status.is_unavailable());
        assert!(!ready_future.eligible_wallet_period_ready);
        assert_eq!(
            ready_future.status.error_code(),
            "PBFT_FINAL_CHAIN_WALLET_AGGREGATE_PERIOD_MISMATCH"
        );

        let unavailable = service
            .collect_dpos_wallet_aggregate_vote_count(
                &final_chain,
                crate::pbft_vote_generation::PbftFinalChainDposWalletAggregateVoteCountRequest {
                    period: 99,
                    eligible_wallet_period: 99,
                    addresses: vec![H160::from(validator)],
                },
            )
            .expect("future aggregate vote lookup should be returned as unavailable data");
        assert_eq!(unavailable.status.as_u8(), 1);
        assert!(unavailable.status.is_unavailable());
        assert!(unavailable.eligible_wallet_period_ready);
        assert_eq!(
            unavailable.status.error_code(),
            "PBFT_FINAL_CHAIN_WALLET_VOTES_FUTURE_PERIOD"
        );
    }

    #[test]
    fn composed_service_collects_pbft_dpos_wallet_eligibility() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let validator = voter_from_secret(&NODE_SECRET);
        let ready = service
            .collect_dpos_wallet_eligibility(
                &final_chain,
                crate::pbft_vote_generation::PbftFinalChainDposWalletEligibilityRequest {
                    period: 0,
                    address: H160::from(validator),
                },
            )
            .expect("ready wallet eligibility lookup should return data");
        assert_eq!(ready.status.as_u8(), 0);
        assert!(ready.status.is_ready());
        assert!(ready.is_eligible());
        assert_eq!(ready.vote_count(), 5);

        let zero_stake = service
            .collect_dpos_wallet_eligibility(
                &final_chain,
                crate::pbft_vote_generation::PbftFinalChainDposWalletEligibilityRequest {
                    period: 0,
                    address: H160::from([0xA1; 20]),
                },
            )
            .expect("zero-stake wallet lookup should be ready with zero values");
        assert_eq!(zero_stake.status.as_u8(), 0);
        assert!(zero_stake.status.is_ready());
        assert!(!zero_stake.is_eligible());
        assert_eq!(zero_stake.vote_count(), 0);
    }

    #[test]
    fn composed_service_collects_pbft_dpos_wallet_eligibility_batch() {
        let (service, final_chain) = service_with_test_chain(5_000, NODE_SECRET);
        let validator = voter_from_secret(&NODE_SECRET);
        let ready = service
            .collect_dpos_wallet_eligibility_batch(
                &final_chain,
                crate::pbft_vote_generation::PbftFinalChainDposWalletEligibilityBatchRequest {
                    period: 0,
                    addresses: vec![H160::from(validator), H160::from([0xA1; 20])],
                },
            )
            .expect("ready wallet batch lookup should return data");
        assert_eq!(ready.status.as_u8(), 0);
        assert!(ready.status.is_ready());
        assert_eq!(ready.address_facts.len(), 2);
        assert_eq!(ready.address_facts[0].status.as_u8(), 0);
        assert_eq!(ready.address_facts[1].status.as_u8(), 0);
        assert!(ready.address_facts[0].is_eligible());
        assert_eq!(ready.address_facts[0].vote_count(), 5);
        assert!(!ready.address_facts[1].is_eligible());
        assert_eq!(ready.address_facts[1].vote_count(), 0);

        let empty_future = service
            .collect_dpos_wallet_eligibility_batch(
                &final_chain,
                crate::pbft_vote_generation::PbftFinalChainDposWalletEligibilityBatchRequest {
                    period: 99,
                    addresses: Vec::new(),
                },
            )
            .expect("empty batches do not require period state");
        assert!(empty_future.status.is_ready());
        assert!(empty_future.address_facts.is_empty());

        let unavailable = service
            .collect_dpos_wallet_eligibility_batch(
                &final_chain,
                crate::pbft_vote_generation::PbftFinalChainDposWalletEligibilityBatchRequest {
                    period: 99,
                    addresses: vec![H160::from(validator), H160::from([0xA1; 20])],
                },
            )
            .expect("future batch lookup should be returned as unavailable data");
        assert_eq!(unavailable.status.as_u8(), 1);
        assert!(unavailable.status.is_unavailable());
        assert_eq!(unavailable.address_facts[0].status.as_u8(), 1);
        assert_eq!(
            unavailable.address_facts[0].status.error_code(),
            "PBFT_FINAL_CHAIN_ADDRESS_FACT_FUTURE_PERIOD"
        );
    }

    fn cert_vote_rlp(block_hash: H256, period: u64, secret: [u8; 32]) -> Vec<u8> {
        generate_pbft_vote(PbftVoteGenerationInput {
            block_hash,
            vote_type: PbftVoteType::Cert,
            period,
            round: 2,
            step: 3,
            node_secret: secret,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&secret).into(),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap()
        .vote_rlp
    }

    fn seed_reward_cert_votes(service: &PbftService, block_hash: H256, period: u64) -> Vec<H256> {
        let mut vote_hashes = Vec::new();
        for secret in [NODE_SECRET, NODE_SECRET_TWO] {
            let vote_rlp = cert_vote_rlp(block_hash, period, secret);
            let validation = validate_canonical_pbft_vote(
                &vote_rlp,
                PbftVoteValidationExternalFacts {
                    voter_dpos_ready: true,
                    voter_dpos_vote_count: 40,
                    total_dpos_ready: true,
                    total_dpos_vote_count: 100,
                    future_dpos_state: false,
                    unknown_error: false,
                    vrf_key_ready: true,
                    has_vrf_key: true,
                    vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
                    strict_vrf: true,
                    committee_size: 100,
                    number_of_proposers: 20,
                    has_preverified_weight: false,
                    preverified_weight: 0,
                },
            )
            .unwrap();
            service
                .verified_votes()
                .lock()
                .unwrap()
                .admit_validated_vote(
                    &vote_rlp,
                    &validation,
                    PbftVoteEventFactFlags {
                        vote_already_known: false,
                        carries_proposed_block: true,
                        valid_stale_reward_vote: false,
                    },
                    PbftVoteProgressContext {
                        current_period: period,
                        current_round: 2,
                        max_future_period_delta: 0,
                        two_t_plus_one_threshold: Some(80),
                        require_proposed_block_sidecar: false,
                        slashing_enabled: true,
                    },
                )
                .unwrap();
            vote_hashes.push(validation.vote_hash);
        }
        vote_hashes
    }

    fn reward_finalization_start_request(block_hash: H256) -> PbftFinalizationExecutorStartRequest {
        use crate::pbft_finalize::{
            PbftFinalizationAnchor, PbftFinalizationCleanupIntent, PbftFinalizationPlan,
            PbftFinalizationStatus, PbftFinalizationStorageWriteIntent,
            PbftFinalizationStorageWriteStage,
        };
        use crate::pbft_manager::PbftFinalizationExecutorStartMode;

        PbftFinalizationExecutorStartRequest {
            plan: PbftFinalizationPlan {
                finalize_block: true,
                anchor: PbftFinalizationAnchor::Anchored,
                executed_pbft_block: false,
                cleanup: PbftFinalizationCleanupIntent {
                    persist_pbft_block_metadata: true,
                    reset_reward_votes: true,
                    set_dag_block_order: false,
                    update_sortition_params: false,
                    update_finalized_transactions_status: false,
                    update_pbft_chain: false,
                    clear_anchor_dag_cache: false,
                    finalize_final_chain: false,
                    maybe_update_dynamic_lambda: false,
                    advance_period: false,
                    process_pillar_block: false,
                },
                storage_write_intent: PbftFinalizationStorageWriteIntent {
                    persist_pbft_head: true,
                    persist_period_data: false,
                    reset_reward_votes: true,
                    update_sortition_params: false,
                    apply_dynamic_lambda_update: false,
                    persist_period_lambda: false,
                    persist_executed_pbft_status: false,
                    process_pillar_block: false,
                    pbft_block_hash: block_hash,
                    pbft_head_hash: block_hash,
                    block_period: 12,
                    null_anchor: false,
                    anchor_hash: H256::zero(),
                    reward_vote_period: 12,
                    reward_vote_round: 2,
                    reward_vote_step: 3,
                    reward_vote_block_hash: block_hash,
                    period_lambda: 0,
                    blocks_per_year: 0,
                    rounds_count_dynamic_lambda: 0,
                    dynamic_lambda: 0,
                    executed_pbft_status: false,
                    pbft_head_payload: vec![0xde, 0xad, 0xbe, 0xef],
                    period_data_rlp: Vec::new(),
                    dag_block_period_writes: Vec::new(),
                    transaction_location_writes: Vec::new(),
                },
                status: PbftFinalizationStatus::Accepted,
            },
            mode: PbftFinalizationExecutorStartMode::Fresh {
                primary_stages: vec![PbftFinalizationStorageWriteStage::default()],
                sync: false,
            },
        }
    }

    fn install_finalization_executor(
        service: &PbftService,
        block_period: u64,
        cleanup: crate::pbft_finalize::PbftFinalizationCleanupIntent,
        actions: Vec<crate::pbft_finalize::PbftFinalizationRuntimeAction>,
    ) {
        use crate::pbft_finalize::{
            PbftFinalizationAnchor, PbftFinalizationPlan, PbftFinalizationRuntimePlan,
            PbftFinalizationStatus, PbftFinalizationStorageWriteIntent,
            start_pbft_finalization_runtime,
        };

        let plan = PbftFinalizationPlan {
            finalize_block: true,
            anchor: PbftFinalizationAnchor::Anchored,
            executed_pbft_block: false,
            cleanup,
            storage_write_intent: PbftFinalizationStorageWriteIntent {
                persist_pbft_head: false,
                persist_period_data: false,
                reset_reward_votes: false,
                update_sortition_params: false,
                apply_dynamic_lambda_update: false,
                persist_period_lambda: false,
                persist_executed_pbft_status: false,
                process_pillar_block: false,
                pbft_block_hash: H256::repeat_byte(7),
                pbft_head_hash: H256::repeat_byte(8),
                block_period,
                null_anchor: false,
                anchor_hash: H256::repeat_byte(4),
                reward_vote_period: block_period,
                reward_vote_round: 2,
                reward_vote_step: 3,
                reward_vote_block_hash: H256::repeat_byte(7),
                period_lambda: 0,
                blocks_per_year: 777,
                rounds_count_dynamic_lambda: 0,
                dynamic_lambda: 0,
                executed_pbft_status: false,
                pbft_head_payload: Vec::new(),
                period_data_rlp: Vec::new(),
                dag_block_period_writes: Vec::new(),
                transaction_location_writes: Vec::new(),
            },
            status: PbftFinalizationStatus::Accepted,
        };
        let runtime_plan = PbftFinalizationRuntimePlan {
            finalize_block: true,
            status: PbftFinalizationStatus::Accepted,
            actions,
        };
        let mut manager = service.manager_state();
        manager.finalization_runtime_session = Some(start_pbft_finalization_runtime(&runtime_plan));
        manager.finalization_runtime_plan = Some(plan);
    }

    #[test]
    fn restore_derives_period_and_cacti_activation_from_chain() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_derivation");

        let active = PbftService::restore(storage.clone(), config(0)).unwrap();
        let active_snapshot = active.manager_state().state.snapshot();
        assert_eq!(active_snapshot.period, 1);
        assert_eq!(active_snapshot.current_round_lambda_ms, 1_500);

        let inactive = PbftService::restore(storage, config(1)).unwrap();
        let inactive_snapshot = inactive.manager_state().state.snapshot();
        assert_eq!(inactive_snapshot.period, 1);
        assert_eq!(inactive_snapshot.current_round_lambda_ms, 100);

        drop(active);
        drop(inactive);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn proposed_block_effect_derives_identity_and_preserves_duplicate_semantics() {
        let (path, storage) = temp_storage("rustaxa_pbft_proposed_effect");
        let service = PbftService::restore(storage.clone(), config(10)).unwrap();
        let (block_rlp, link) = pbft_block_rlp(2, 7);

        assert!(
            service
                .publish_proposed_block_effect(block_rlp.clone())
                .unwrap()
        );
        let published = service
            .proposed_block(link.period, link.block_hash)
            .expect("derived proposal identity is indexed");
        assert_eq!(published.pivot_hash, link.pivot_dag_block_hash);
        assert_eq!(published.block_rlp, block_rlp);
        assert!(
            !service
                .publish_proposed_block_effect(published.block_rlp.clone())
                .unwrap()
        );
        assert!(service.publish_proposed_block_effect(vec![0x01]).is_err());
        assert_eq!(service.proposed_blocks().snapshot_entries().len(), 1);

        let restored = PbftService::restore(storage, config(10)).unwrap();
        assert!(
            restored
                .proposed_block(link.period, link.block_hash)
                .is_some()
        );

        drop(restored);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn proposed_block_admission_returns_typed_missing_result() {
        let (path, storage) = temp_storage("rustaxa_pbft_proposed_admission_missing");
        let service = PbftService::restore(storage.clone(), config(10)).unwrap();
        let final_chain = final_chain_with_pillar_voters(storage.clone(), &[]);
        let dag = dag_service(storage);

        let result = service
            .admit_proposed_block(
                &final_chain,
                &dag,
                PbftProposedBlockAdmissionRequest {
                    period: 2,
                    block_hash: H256::repeat_byte(2),
                    pbft_gas_limit: 1_000_000,
                    extra_data_required: false,
                    pillar_block_required: false,
                },
            )
            .unwrap();

        assert_eq!(result.status, PbftProposedBlockAdmissionStatus::Missing);
        assert!(result.block_rlp.is_empty());
        drop(final_chain);
        drop(service);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn proposed_block_admission_returns_cached_canonical_block_without_revalidation() {
        let (path, storage) = temp_storage("rustaxa_pbft_proposed_admission_cached");
        let service = PbftService::restore(storage.clone(), config(10)).unwrap();
        let final_chain = final_chain_with_pillar_voters(storage.clone(), &[]);
        let dag = dag_service(storage);
        let (block_rlp, link) = proposed_admission_block_rlp(2, H256::zero(), H256::zero());
        service
            .publish_proposed_block(
                link.period,
                link.block_hash,
                link.pivot_dag_block_hash,
                block_rlp.clone(),
            )
            .unwrap();
        service
            .mark_proposed_block_valid(link.period, link.block_hash)
            .unwrap();

        let result = service
            .admit_proposed_block(
                &final_chain,
                &dag,
                PbftProposedBlockAdmissionRequest {
                    period: link.period,
                    block_hash: link.block_hash,
                    pbft_gas_limit: 1_000_000,
                    extra_data_required: false,
                    pillar_block_required: false,
                },
            )
            .unwrap();

        assert_eq!(
            result.status,
            PbftProposedBlockAdmissionStatus::AcceptedAlreadyValid
        );
        assert_eq!(result.block_rlp, block_rlp);
        drop(final_chain);
        drop(service);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn proposed_block_admission_rejects_malformed_timestamp_and_signature_without_cache_mutation() {
        let (path, storage) = temp_storage("rustaxa_pbft_proposed_admission_malformed");
        let service = PbftService::restore(storage.clone(), config(10)).unwrap();
        let final_chain = final_chain_with_pillar_voters(storage.clone(), &[]);
        let dag = dag_service(storage);

        for (period, invalid_timestamp, valid_signature) in [(2, true, true), (3, false, false)] {
            let (block_rlp, link) = proposed_admission_block_rlp_with_shape(
                period,
                H256::zero(),
                H256::zero(),
                invalid_timestamp,
                valid_signature,
            );
            service
                .publish_proposed_block(
                    link.period,
                    link.block_hash,
                    link.pivot_dag_block_hash,
                    block_rlp,
                )
                .unwrap();
            let result = service.admit_proposed_block(
                &final_chain,
                &dag,
                PbftProposedBlockAdmissionRequest {
                    period: link.period,
                    block_hash: link.block_hash,
                    pbft_gas_limit: 1_000_000,
                    extra_data_required: false,
                    pillar_block_required: false,
                },
            );
            assert!(result.is_err());
            assert!(
                !service
                    .proposed_block(link.period, link.block_hash)
                    .expect("malformed proposal remains indexed but invalid")
                    .is_valid
            );
        }

        drop(final_chain);
        drop(service);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn proposed_block_extra_data_accepts_non_utf8_implementation_bytes() {
        let mut extra = RlpStream::new_list(6);
        extra.append(&1_u16);
        extra.append(&2_u16);
        extra.append(&3_u16);
        extra.append(&4_u16);
        extra.append(&vec![0xff_u8, 0xfe]);
        extra.append_empty_data();
        let mut block = RlpStream::new_list(9);
        block.append(&H256::zero());
        block.append(&H256::zero());
        block.append(&H256::zero());
        block.append(&H256::zero());
        block.append(&1_u64);
        block.append(&0_u64);
        block.begin_list(0);
        block.append(&extra.out().to_vec());
        block.append(&vec![0_u8; 65]);
        let block = block.out().to_vec();

        assert_eq!(
            decode_pbft_proposed_block_extra_data(&Rlp::new(&block)).unwrap(),
            (true, None)
        );
    }

    #[test]
    fn restore_constructs_one_shared_network_service_with_direct_empty_egress() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_network_service");
        let service = PbftService::restore(storage, config(10)).expect("service restores");
        let first = service.network_service();
        let second = service.network_service();

        let next = first
            .ingest_pbft_next_votes_bundle_request(NetworkPbftNextVotesBundleRequest {
                transport_lane: 6,
                peer_id: [7; 64],
                peer_period: 1,
                peer_round: 1,
                source_payload_id: 90,
            })
            .unwrap();
        assert_eq!(
            next.status,
            crate::network_api::NETWORK_INGRESS_STATUS_NEXT_VOTES_NO_PREVIOUS_ROUND
        );
        assert_eq!(next.queued_effect_count, 0);
        assert!(second.drain_work(6, 10).unwrap().effects.is_empty());

        service.complete_pillar_bootstrap().unwrap();
        let pillar = second
            .ingest_get_pillar_votes_bundle_request(NetworkGetPillarVotesBundleRequest {
                transport_lane: 6,
                peer_id: [8; 64],
                period: 11,
                pillar_block_hash: H256::from_low_u64_be(91).into(),
                source_payload_id: 91,
            })
            .unwrap();
        assert_eq!(pillar.status, NETWORK_INGRESS_STATUS_PILLAR_VOTES_NO_DATA);
        assert_eq!(pillar.queued_effect_count, 0);
        assert!(first.drain_work(6, 10).unwrap().effects.is_empty());

        drop(first);
        drop(second);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn restore_rejects_invalid_pillar_schedule_before_publication() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_network_schedule");
        let mut invalid = config(10);
        invalid.pillar_blocks_interval = 1;
        let error = match PbftService::restore(storage, invalid) {
            Ok(_) => panic!("invalid pillar interval should fail"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("PILLAR_BLOCKS_INTERVAL_MUST_EXCEED_ONE")
        );
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn restore_accepts_disabled_ficus_with_zero_interval_and_rejects_requests_safely() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_disabled_ficus_schedule");
        let mut disabled = config(10);
        disabled.ficus_activation_period = u64::MAX;
        disabled.pillar_blocks_interval = 0;
        let service = PbftService::restore(storage, disabled).expect("disabled Ficus restores");

        let decision = service
            .network_service()
            .ingest_get_pillar_votes_bundle_request(NetworkGetPillarVotesBundleRequest {
                transport_lane: 6,
                peer_id: [8; 64],
                period: 11,
                pillar_block_hash: H256::from_low_u64_be(91).into(),
                source_payload_id: 91,
            })
            .expect("disabled Ficus request is rejected without interval arithmetic");

        assert_eq!(
            decision.status,
            NETWORK_INGRESS_STATUS_PILLAR_VOTES_INACTIVE
        );
        assert_eq!(decision.queued_effect_count, 2);

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn service_owns_period_data_queue_lifecycle() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_period_queue");
        let service = PbftService::restore(storage, config(10)).expect("service restores");
        let entry = PeriodDataQueueEntryRef {
            period_data_rlp: vec![0x11],
            source_peer_id: [0x01; 64],
            period: 1,
            block_hash: H256::repeat_byte(0x11),
            prev_block_hash: H256::repeat_byte(0x22),
            pivot_hash: H256::repeat_byte(0x33),
            final_chain_hash: H256::repeat_byte(0x44),
            reward_vote_hashes: vec![H256::repeat_byte(0x55)],
            pillar_vote_rlps: vec![vec![0xa1]],
            transaction_rlps: vec![vec![0xb1]],
            previous_cert_vote_rlps: vec![vec![0xc1]],
            dag_transaction_hashes: vec![H256::repeat_byte(0x66)],
            period_data_transaction_hashes: vec![H256::repeat_byte(0x77)],
            period_data_transaction_identities: vec![],
            previous_cert_votes_present: true,
            previous_cert_first_vote_has_weight: false,
            pillar_votes_present: true,
            extra_data_present: true,
            extra_data_pillar_block_hash_present: false,
        };

        let pushed = service
            .push_period_data_queue(PeriodDataQueuePushRequest {
                entry: entry.clone(),
                max_pbft_size: 0,
                current_block_cert_vote_rlps: vec![vec![0xd1]],
            })
            .expect("queue push succeeds");
        assert!(pushed.accepted);
        assert_eq!(
            service
                .period_data_queue_snapshot()
                .expect("native chain-backed queue snapshot succeeds"),
            PeriodDataQueueSnapshot {
                period: 1,
                syncing_period: 1,
                last_block_hash_or_chain: entry.block_hash,
                size: 1,
                empty: false,
            }
        );
        let manager = service.manager_snapshot();
        assert_eq!(
            service.application_status_snapshot().unwrap(),
            PbftApplicationStatusSnapshot {
                period: manager.period,
                round: manager.round,
                step: manager.step,
                finalized_chain_size: 0,
                syncing_period: 1,
                sync_queue_size: 1,
            }
        );
        let advanced_hash = H256::repeat_byte(0xaa);
        service
            .pbft_chain_update(advanced_hash, H256::repeat_byte(0xbb))
            .expect("native PBFT chain advances");
        let advanced_snapshot = service
            .period_data_queue_snapshot()
            .expect("queue snapshot samples advanced native chain");
        assert_eq!(advanced_snapshot.syncing_period, 1);
        assert_eq!(advanced_snapshot.last_block_hash_or_chain, advanced_hash);

        let popped = service.pop_period_data_queue().expect("queue pop succeeds");
        assert_eq!(popped.cert_vote_rlps, vec![vec![0xd1]]);
        assert!(popped.use_last_block_cert_votes);
        assert_eq!(service.clean_old_period_data_queue(2), 0);
        service.clear_period_data_queue();
        assert!(
            service
                .period_data_queue_snapshot()
                .expect("cleared native chain-backed queue snapshot succeeds")
                .empty
        );

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn queue_drain_applies_native_stale_cleanup_before_returning_external_step() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_queue_drain_cleanup");
        let service = PbftService::restore(storage, config(10)).expect("service restores");
        let stale_hash = H256::repeat_byte(0x11);
        service
            .push_period_data_queue(PeriodDataQueuePushRequest {
                entry: PeriodDataQueueEntryRef {
                    period_data_rlp: vec![0x11],
                    source_peer_id: [0x01; 64],
                    period: 1,
                    block_hash: stale_hash,
                    prev_block_hash: H256::repeat_byte(0x22),
                    pivot_hash: H256::repeat_byte(0x33),
                    final_chain_hash: H256::repeat_byte(0x44),
                    reward_vote_hashes: vec![],
                    pillar_vote_rlps: vec![],
                    transaction_rlps: vec![],
                    previous_cert_vote_rlps: vec![],
                    dag_transaction_hashes: vec![],
                    period_data_transaction_hashes: vec![],
                    period_data_transaction_identities: vec![],
                    previous_cert_votes_present: false,
                    previous_cert_first_vote_has_weight: false,
                    pillar_votes_present: false,
                    extra_data_present: false,
                    extra_data_pillar_block_hash_present: false,
                },
                max_pbft_size: 0,
                current_block_cert_vote_rlps: vec![],
            })
            .expect("stale queue entry is admitted");
        service
            .pbft_chain_update(H256::repeat_byte(0xaa), H256::repeat_byte(0xbb))
            .expect("native PBFT chain advances past queued period");

        service.complete_bootstrap();
        service.begin_pbft_sync_queue_drain();
        let step = service
            .pbft_sync_queue_drain_next()
            .expect("ready service returns a drain step");

        assert_eq!(
            step.action,
            crate::pbft_sync::PbftSyncQueueDrainAction::Stop
        );
        assert!(
            service
                .period_data_queue_snapshot()
                .expect("queue snapshot succeeds")
                .empty
        );

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn service_commits_lifecycle_storage_before_runtime_publication() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_lifecycle_commit");
        storage.pbft().write_manager_field(0, 1).unwrap();
        storage.pbft().write_manager_field(1, 1).unwrap();
        storage.pbft().write_manager_field(2, 1_500).unwrap();
        storage.pbft().write_manager_status(2, true).unwrap();
        storage.pbft().write_manager_status(3, true).unwrap();
        let service = PbftService::restore(storage.clone(), config(1)).expect("service restores");
        crate::pbft_vote_storage::save_own_verified_vote(
            storage.as_ref(),
            crate::pbft_vote_storage::PbftVoteStorageRecord {
                hash: H256::repeat_byte(0xbc),
                vote_rlp: vec![0xc0],
            },
        )
        .expect("own vote persists");

        let outcome = service
            .execute_lifecycle_transition(PbftManagerLifecycleTransitionRequest {
                kind: crate::pbft_manager::PbftManagerTransitionKind::ResetConsensus,
                target_period: 10,
                target_round: 4,
                has_network_next_voting_step: false,
                network_next_voting_step: 0,
            })
            .expect("transition commits");

        assert_eq!(outcome.status, PbftManagerTransitionStorageStatus::Applied);
        assert_eq!((outcome.snapshot.round, outcome.snapshot.step), (4, 1));
        assert!(!outcome.snapshot.already_next_voted_value);
        assert!(!outcome.snapshot.already_next_voted_null);
        assert_eq!(storage.pbft().manager_field(0).unwrap(), Some(4));
        assert_eq!(storage.pbft().manager_field(1).unwrap(), Some(1));
        assert_eq!(storage.pbft().manager_status(2).unwrap(), Some(false));
        assert_eq!(storage.pbft().manager_status(3).unwrap(), Some(false));
        assert!(
            storage
                .pbft()
                .own_verified_vote_records()
                .unwrap()
                .is_empty()
        );
        assert!(outcome.clear_broadcasted_vote_sidecars);
        assert!(outcome.reset_current_round_timer);

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn service_rejects_invalid_lifecycle_facts_without_mutation() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_lifecycle_reject");
        let service = PbftService::restore(storage, config(1)).expect("service restores");
        let before = service.manager_state().state.snapshot();

        let unknown = service
            .execute_lifecycle_transition(PbftManagerLifecycleTransitionRequest {
                kind: crate::pbft_manager::PbftManagerTransitionKind::Unknown,
                target_period: 10,
                target_round: 4,
                has_network_next_voting_step: false,
                network_next_voting_step: 0,
            })
            .expect("unknown kind rejects");
        assert_eq!(unknown.status, PbftManagerTransitionStorageStatus::Rejected);
        assert_eq!(unknown.snapshot, before);
        assert_eq!(unknown.error_code, "PBFT_MANAGER_TRANSITION_UNKNOWN_KIND");
        assert!(!unknown.remove_cert_voted_sidecar);
        assert!(!unknown.clear_broadcasted_vote_sidecars);

        let mismatched_network_step = service
            .execute_lifecycle_transition(PbftManagerLifecycleTransitionRequest {
                kind: crate::pbft_manager::PbftManagerTransitionKind::ToFilter,
                target_period: 10,
                target_round: 4,
                has_network_next_voting_step: true,
                network_next_voting_step: 7,
            })
            .expect("network-step mismatch rejects");
        assert_eq!(mismatched_network_step.snapshot, before);
        assert_eq!(
            mismatched_network_step.error_code,
            "PBFT_MANAGER_TRANSITION_NETWORK_STEP_PRESENCE_MISMATCH"
        );

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn service_owns_manager_status_and_cursor_persistence() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_manager_persistence");
        let service = PbftService::restore(storage.clone(), config(1)).expect("service restores");

        let reset = service.apply_executed_block_reset();
        assert_eq!(reset.status, PbftManagerTransitionStorageStatus::Applied);
        assert_eq!(reset.applied_writes, 1);
        assert!(!reset.snapshot.executed_pbft_block);
        assert_eq!(storage.pbft().manager_status(0).unwrap(), Some(false));

        let next_voted = service.apply_next_voted_status(2).expect("status persists");
        assert!(next_voted.already_next_voted_value);
        assert!(!next_voted.already_next_voted_null);
        assert_eq!(storage.pbft().manager_status(2).unwrap(), Some(true));
        let before_rejected_status = service.manager_state().state.snapshot();
        assert_eq!(
            service.apply_next_voted_status(0).unwrap_err().to_string(),
            "PBFT_MANAGER_NEXT_VOTED_STATUS_UNSUPPORTED"
        );
        assert_eq!(
            service.manager_state().state.snapshot(),
            before_rejected_status
        );

        let before_cursor = service.manager_state().state.snapshot();
        let cursor = service.apply_cursor_field(0, 8).expect("round persists");
        assert_eq!((cursor.round, cursor.step), (8, before_cursor.step));
        assert_eq!(storage.pbft().manager_field(0).unwrap(), Some(8));
        let before_rejected_cursor = service.manager_state().state.snapshot();
        assert!(
            service
                .apply_cursor_field(2, 1)
                .unwrap_err()
                .to_string()
                .contains("unsupported PBFT manager cursor field")
        );
        assert_eq!(
            service.manager_state().state.snapshot(),
            before_rejected_cursor
        );

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn service_owns_manager_scalar_and_storage_tasks() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_scalar_owner");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();

        let broadcast = service.apply_broadcast_counters(2, 3, 4, 5);
        assert_eq!(broadcast.broadcast_votes_counter, 2);
        assert_eq!(broadcast.rebroadcast_votes_counter, 3);
        assert_eq!(broadcast.broadcast_reward_votes_counter, 4);
        assert_eq!(broadcast.rebroadcast_reward_votes_counter, 5);
        assert_eq!(service.manager_snapshot(), broadcast);

        assert!(
            service
                .cached_candidate_dag_payload(
                    &dag_service(storage.clone()),
                    H256::repeat_byte(0x44)
                )
                .unwrap_err()
                .to_string()
                .contains("PBFT_CANDIDATE_DAG_CACHE_MISSING")
        );

        let block_hash = H256::repeat_byte(0x55);
        let cert = service
            .save_cert_voted_block_in_round(12, 3, block_hash, &[0xc0])
            .unwrap();
        assert!(cert.has_cert_voted_block);
        assert_eq!(cert.cert_voted_block_period, 12);
        assert_eq!(cert.cert_voted_block_round, 3);
        assert_eq!(cert.cert_voted_block_hash, block_hash);
        assert!(!service.cert_voted_block_in_round().unwrap().is_empty());

        let sleep = service.plan_runtime_sleep_until_next_step(i64::MAX);
        assert!(sleep.accepted);
        assert!(!sleep.should_sleep);

        assert!(!service.load_startup_replay_period(99, true).unwrap().found);
        assert!(service.own_pillar_block_vote().unwrap().is_empty());
        assert!(
            !service
                .dag_block_period(H256::repeat_byte(0x66))
                .unwrap()
                .found
        );
        assert!(!service.pbft_block_in_db(H256::repeat_byte(0x77)).unwrap());

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn candidate_dag_preparation_validates_then_short_circuits_by_anchor() -> Result<()> {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_candidate_dag_owner");
        let service = PbftService::restore(storage.clone(), config(1))?;
        let dag = dag_service(storage.clone());
        let genesis = H256::repeat_byte(1);
        let block_rlp = candidate_dag_block_rlp(genesis, 42);
        let block = dag_manager_block_from_rlp(&block_rlp)?;
        dag.lock_dag()?.state.add_block(block.clone())?;
        save_dag_block_to_storage(storage.as_ref(), block.hash, 1, 0, &block_rlp)?;
        let prepared = dag
            .prepare_pbft_candidate_payload(1, block.hash)?
            .expect("candidate payload");
        let order_hash = pbft_candidate_dag_order_hash(&prepared.payload);

        assert_eq!(
            service.prepare_candidate_dag(&dag, 1, block.hash, H256::zero(), 100)?,
            PbftCandidateDagPreparationStatus::OrderHashInvalid
        );
        assert!(
            service
                .cached_candidate_dag_payload(&dag, block.hash)
                .is_err()
        );
        assert_eq!(
            service.prepare_candidate_dag(&dag, 1, block.hash, order_hash, 100)?,
            PbftCandidateDagPreparationStatus::Valid
        );
        assert_eq!(
            service.cached_candidate_dag_payload(&dag, block.hash)?,
            prepared.payload
        );
        assert_eq!(
            service.prepare_candidate_dag(&dag, 999, block.hash, H256::zero(), 0)?,
            PbftCandidateDagPreparationStatus::Valid
        );
        assert_eq!(
            service.prepare_candidate_dag(&dag, 1, H256::repeat_byte(9), H256::zero(), 100)?,
            PbftCandidateDagPreparationStatus::Missing
        );

        drop(dag);
        drop(service);
        drop(storage);
        fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn candidate_dag_preparation_rejects_divergent_overweight_order_without_cache() -> Result<()> {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_candidate_dag_weight");
        let service = PbftService::restore(storage.clone(), config(1))?;
        let dag = dag_service(storage.clone());
        let genesis = H256::repeat_byte(1);

        dag.lock_dag()?.state.advance_empty_period(1)?;
        let first_rlp = candidate_dag_block_rlp(genesis, 42);
        let first = dag_manager_block_from_rlp(&first_rlp)?;
        dag.lock_dag()?.state.add_block(first.clone())?;
        save_dag_block_to_storage(storage.as_ref(), first.hash, 1, 0, &first_rlp)?;
        let second_rlp = candidate_dag_block_rlp(genesis, 43);
        let second = dag_manager_block_from_rlp(&second_rlp)?;
        dag.lock_dag()?.state.add_block(second.clone())?;
        save_dag_block_to_storage(storage.as_ref(), second.hash, 1, 0, &second_rlp)?;

        let ghost = dag.dag_ghost_path(crate::dag_transaction_service::DagGhostPathRoot::Block(
            genesis,
        ))?;
        assert!(ghost.len() > 1);
        let divergent = if ghost[1] == first.hash {
            second.hash
        } else {
            first.hash
        };
        let prepared = dag
            .prepare_pbft_candidate_payload(2, divergent)?
            .expect("divergent candidate payload");
        let order_hash = pbft_candidate_dag_order_hash(&prepared.payload);

        let (previous_rlp, previous) = pbft_block_rlp_with_pivot(H256::zero(), genesis, 1);
        storage
            .period()
            .write(1, &period_data_with_pbft_block(&previous_rlp))?;
        storage.period().write_pbft_period(previous.block_hash, 1)?;
        service.pbft_chain_update(previous.block_hash, genesis)?;

        assert_eq!(
            service.prepare_candidate_dag(&dag, 2, divergent, order_hash, 41)?,
            PbftCandidateDagPreparationStatus::WeightInvalid
        );
        assert!(
            service
                .cached_candidate_dag_payload(&dag, divergent)
                .is_err()
        );
        assert_eq!(
            service
                .manager_state()
                .state
                .cached_anchor_dag_order_count(),
            0
        );

        drop(dag);
        drop(service);
        drop(storage);
        fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn proposal_session_native_dag_drain_builds_valid_order() -> Result<()> {
        let (path, storage) = temp_storage("rustaxa_consensus_proposal_native_dag_valid");
        let service = PbftService::restore(storage.clone(), config(1))?;
        let dag = dag_service(storage.clone());
        let genesis = H256::repeat_byte(1);
        let block_rlp = candidate_dag_block_rlp(genesis, 42);
        let block = dag_manager_block_from_rlp(&block_rlp)?;
        dag.lock_dag()?.state.add_block(block.clone())?;
        save_dag_block_to_storage(storage.as_ref(), block.hash, 1, 0, &block_rlp)?;
        let prepared = dag
            .prepare_pbft_candidate_payload(1, block.hash)?
            .expect("proposal order");
        let expected_order_hash = pbft_candidate_dag_order_hash(&prepared.payload);

        service.complete_bootstrap();
        service.begin_proposal_session(native_proposal_fact(
            1,
            genesis,
            vec![genesis, block.hash],
            100,
        ));
        let step = service
            .proposal_session_next_with_dag(&dag)?
            .expect("proposal step");
        assert_eq!(step.action, PbftManagerProposalAction::BuildProposal);
        assert_eq!(step.anchor_hash, block.hash);
        assert_eq!(step.order_hash, expected_order_hash);
        assert_eq!(step.dag_blocks_included, 1);
        assert_eq!(step.eligible_wallet_indices, vec![7]);

        drop(dag);
        drop(service);
        drop(storage);
        fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn proposal_session_stale_generation_preserves_same_and_different_period_replacements() {
        let (path, storage) = temp_storage("rustaxa_consensus_proposal_generation");
        let service = PbftService::restore(storage.clone(), config(1)).expect("PBFT service");
        let genesis = H256::repeat_byte(1);
        let anchor = H256::repeat_byte(0x93);
        service.complete_bootstrap();

        for replacement_period in [1, 2] {
            service.begin_proposal_session(native_proposal_fact(
                1,
                genesis,
                vec![genesis, anchor],
                100,
            ));
            let (generation, request) = {
                let mut manager = service.manager_state();
                let generation = manager.proposal_session_generation;
                let request = next_pbft_manager_proposal_session(
                    manager.proposal_session.as_mut().expect("original cursor"),
                );
                (generation, request)
            };
            assert_eq!(request.action, PbftManagerProposalAction::RequestDagOrder);

            service.begin_proposal_session(native_proposal_fact(
                replacement_period,
                genesis,
                vec![genesis, anchor],
                100,
            ));
            let stale = service
                .report_proposal_dag_order_for_generation(
                    generation,
                    PbftManagerProposalDagOrderReport {
                        anchor_hash: request.requested_anchor_hash,
                        dag_blocks: Vec::new(),
                        order_available: false,
                    },
                )
                .expect("stale result");
            assert_eq!(stale.action, PbftManagerProposalAction::ContractError);
            assert_eq!(stale.error_code, "PBFT_MANAGER_PROPOSAL_STALE_CURSOR");

            let replacement = service
                .proposal_session_next()
                .expect("replacement survives");
            assert_eq!(
                replacement.action,
                PbftManagerProposalAction::RequestDagOrder
            );
            assert_eq!(replacement.requested_anchor_hash, anchor);
        }

        drop(service);
        drop(storage);
        fs::remove_dir_all(path).expect("temporary storage cleanup");
    }

    #[test]
    fn proposal_session_native_dag_drain_reports_missing_order() -> Result<()> {
        let (path, storage) = temp_storage("rustaxa_consensus_proposal_native_dag_missing");
        let service = PbftService::restore(storage.clone(), config(1))?;
        let dag = dag_service(storage.clone());
        let genesis = H256::repeat_byte(1);
        let missing = H256::repeat_byte(0x91);

        service.complete_bootstrap();
        service.begin_proposal_session(native_proposal_fact(
            1,
            genesis,
            vec![genesis, missing],
            100,
        ));
        let step = service
            .proposal_session_next_with_dag(&dag)?
            .expect("missing-order step");
        assert_eq!(step.action, PbftManagerProposalAction::SkipProposal);
        assert_eq!(
            step.status,
            crate::pbft_manager::PbftManagerProposalStatus::MissingDagOrder
        );
        assert_eq!(step.error_code, "PBFT_MANAGER_PROPOSAL_MISSING_DAG_ORDER");

        drop(dag);
        drop(service);
        drop(storage);
        fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn proposal_session_native_dag_drain_reanchors_and_recomputes_order() -> Result<()> {
        let (path, storage) = temp_storage("rustaxa_consensus_proposal_native_dag_reanchor");
        let service = PbftService::restore(storage.clone(), config(1))?;
        let dag = dag_service(storage.clone());
        let genesis = H256::repeat_byte(1);
        let first_rlp = candidate_dag_block_rlp_at_level(genesis, 1, 40);
        let first = dag_manager_block_from_rlp(&first_rlp)?;
        dag.lock_dag()?.state.add_block(first.clone())?;
        save_dag_block_to_storage(storage.as_ref(), first.hash, 1, 0, &first_rlp)?;
        let second_rlp = candidate_dag_block_rlp_at_level(first.hash, 2, 80);
        let second = dag_manager_block_from_rlp(&second_rlp)?;
        dag.lock_dag()?.state.add_block(second.clone())?;
        save_dag_block_to_storage(storage.as_ref(), second.hash, 2, 0, &second_rlp)?;
        let first_prepared = dag
            .prepare_pbft_candidate_payload(1, first.hash)?
            .expect("reanchored order");
        let expected_order_hash = pbft_candidate_dag_order_hash(&first_prepared.payload);

        service.complete_bootstrap();
        service.begin_proposal_session(native_proposal_fact(
            1,
            genesis,
            vec![genesis, first.hash, second.hash],
            50,
        ));
        let step = service
            .proposal_session_next_with_dag(&dag)?
            .expect("reanchored build step");
        assert_eq!(step.action, PbftManagerProposalAction::BuildProposal);
        assert_eq!(step.anchor_hash, first.hash);
        assert_eq!(step.order_hash, expected_order_hash);
        assert_eq!(step.dag_blocks_included, 1);

        drop(dag);
        drop(service);
        drop(storage);
        fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn proposal_session_native_dag_error_preserves_pending_request() -> Result<()> {
        let (path, storage) = temp_storage("rustaxa_consensus_proposal_native_dag_error");
        let service = PbftService::restore(storage.clone(), config(1))?;
        let dag = dag_service(storage.clone());
        let genesis = H256::repeat_byte(1);
        let malformed_hash = H256::repeat_byte(0x92);
        dag.lock_dag()?.state.add_block(DagManagerBlock {
            hash: malformed_hash,
            pivot: genesis,
            tips: Vec::new(),
            level: 1,
            difficulty: 1,
        })?;
        save_dag_block_to_storage(storage.as_ref(), malformed_hash, 1, 0, &[0x80])?;

        service.complete_bootstrap();
        service.begin_proposal_session(native_proposal_fact(
            1,
            genesis,
            vec![genesis, malformed_hash],
            100,
        ));
        let error = service
            .proposal_session_next_with_dag(&dag)
            .expect_err("malformed canonical block must abort native drain");
        assert!(error.to_string().contains("PBFT_PROPOSAL_DAG_BLOCK_DECODE"));
        let pending = service
            .proposal_session_next()
            .expect("pending request remains retryable");
        assert_eq!(pending.action, PbftManagerProposalAction::RequestDagOrder);
        assert_eq!(pending.requested_anchor_hash, malformed_hash);

        drop(dag);
        drop(service);
        drop(storage);
        fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn dynamic_lambda_planning_and_storage_lookup_are_owned_by_native_service() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_dynamic_lambda");
        storage
            .metadata()
            .write_period_lambda(19, 1_234)
            .expect("period lambda persists");
        storage
            .metadata()
            .write_period_lambda(0, 999)
            .expect("period-zero lambda persists for lower-bound regression");
        let service = PbftService::restore(storage, config(1)).unwrap();

        let decision = service
            .plan_finalization_dynamic_lambda(dynamic_lambda_fact(20))
            .expect("dynamic-lambda decision succeeds");
        assert_eq!(decision.plan.status, PbftFinalizationStatus::Accepted);
        assert!(decision.plan.apply_dynamic_lambda_update);
        assert_eq!(decision.plan.period_lambda, 1_500);
        assert_eq!(decision.plan.blocks_per_year, 9_275_294);
        assert_eq!(decision.plan.rounds_count_dynamic_lambda, 0);
        assert_eq!(decision.plan.dynamic_lambda, 1_490);
        assert_eq!(
            decision.last_saved_period_lambda,
            PbftFinalizationPeriodLambdaLookup {
                found: true,
                value: 1_234,
            }
        );

        let missing = service
            .plan_finalization_dynamic_lambda(dynamic_lambda_fact(0))
            .expect("period zero has no prior lambda");
        assert_eq!(
            missing.last_saved_period_lambda,
            PbftFinalizationPeriodLambdaLookup {
                found: false,
                value: 0,
            }
        );

        let mut inactive_fact = dynamic_lambda_fact(20);
        inactive_fact.dynamic_lambda_active = false;
        let inactive = service
            .plan_finalization_dynamic_lambda(inactive_fact)
            .expect("inactive dynamic-lambda decision succeeds");
        assert!(!inactive.plan.apply_dynamic_lambda_update);
        assert!(!inactive.last_saved_period_lambda.found);

        let mut rejected_fact = dynamic_lambda_fact(20);
        rejected_fact.config.lambda_change_interval = 0;
        let rejected = service
            .plan_finalization_dynamic_lambda(rejected_fact)
            .expect("rejected policy does not read prior lambda");
        assert_eq!(rejected.plan.status, PbftFinalizationStatus::ContractError);
        assert!(!rejected.last_saved_period_lambda.found);

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn bootstrap_readiness_is_pending_then_monotonic() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_readiness");
        let service = PbftService::restore(storage, config(1)).unwrap();

        assert!(!service.is_ready());
        service.complete_bootstrap();
        assert!(service.is_ready());
        service.complete_bootstrap();
        assert!(service.is_ready());

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_ready_tracks_final_chain_delay() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_finalization_ready");
        let final_chain = final_chain_with_pillar_voters_and_delay(storage.clone(), &[], 2);
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();

        assert!(service.finalization_ready(&final_chain).unwrap());
        assert_eq!(service.pbft_chain_head().size, 0);

        for index in 1..=3 {
            service
                .pbft_chain_update(H256::from_low_u64_be(index), H256::zero())
                .unwrap();
        }
        assert!(!service.finalization_ready(&final_chain).unwrap());

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_ready_rejects_height_overflow() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_finalization_overflow");
        let final_chain = final_chain_with_pillar_voters_and_delay(storage.clone(), &[], 1);
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(
                &mut batch,
                Column::FinalChainMeta,
                &FinalChain::DB_META_LAST_NUMBER.to_le_bytes(),
                &u64::MAX.to_le_bytes(),
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();

        assert_eq!(
            service
                .finalization_ready(&final_chain)
                .unwrap_err()
                .to_string(),
            "PBFT_MANAGER_FINALIZATION_WAIT_READY_HEIGHT_OVERFLOW"
        );

        drop(service);
        drop(final_chain);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn service_owns_slashing_plan_and_submission_lifecycle() {
        use crate::slashing::{
            DoubleVotingProofInput, DoubleVotingProofPlanStatus, SlashingSubmitterFact,
        };

        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_slashing_owner");
        let mut service_config = config(1);
        service_config.magnolia_activation_period = 10;
        let service = PbftService::restore(storage, service_config).unwrap();
        let input = DoubleVotingProofInput {
            vote_a_hash: H256::repeat_byte(1),
            vote_b_hash: H256::repeat_byte(2),
            vote_a_period: 10,
            vote_b_period: 10,
            vote_a_round: 2,
            vote_b_round: 2,
            vote_a_step: 3,
            vote_b_step: 3,
            vote_a_rlp: vec![0xc1, 0x01],
            vote_b_rlp: vec![0xc1, 0x02],
            submitters: vec![SlashingSubmitterFact {
                wallet_index: 0,
                nonce: U256::one(),
                balance: U256::one(),
            }],
        };

        let mut before_activation = input.clone();
        before_activation.vote_a_period = 9;
        before_activation.vote_b_period = 9;
        assert_eq!(
            service
                .slashing
                .plan_double_voting_proof(before_activation)
                .unwrap()
                .status,
            DoubleVotingProofPlanStatus::BeforeMagnoliaActivation
        );

        let plan = service
            .slashing
            .plan_double_voting_proof(input.clone())
            .unwrap();
        assert_eq!(plan.status, DoubleVotingProofPlanStatus::Planned);
        assert!(
            !service
                .report_verified_vote_slashing_transaction_submission(plan.proof_hash, false)
                .unwrap()
                .submitted
        );
        assert_eq!(
            service
                .slashing
                .plan_double_voting_proof(input.clone())
                .unwrap()
                .status,
            DoubleVotingProofPlanStatus::Planned
        );
        assert!(
            service
                .report_verified_vote_slashing_transaction_submission(plan.proof_hash, true)
                .unwrap()
                .submitted
        );
        assert_eq!(
            service
                .slashing
                .plan_double_voting_proof(input)
                .unwrap()
                .status,
            DoubleVotingProofPlanStatus::DuplicateProof
        );

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn service_owns_manager_executor_session_cursors() {
        use crate::pbft_manager::{
            PbftManagerProposalInitialFact, PbftManagerRuntimeStateCode, PbftManagerRuntimeStatus,
            PbftManagerRuntimeTickFact, PbftManagerStateActionFact,
        };

        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_session_owner");
        let service = PbftService::restore(storage, config(1)).unwrap();
        let tick = PbftManagerRuntimeTickFact {
            tick_id: 42,
            state: PbftManagerRuntimeStateCode::Filter,
            period: 10,
            round: 2,
            step: 3,
            network_available: true,
            network_pbft_syncing: false,
            has_eligible_wallet: true,
            polling_interval_ms: 100,
        };
        let proposal = PbftManagerProposalInitialFact {
            period: 10,
            round: 2,
            previous_pbft_block_hash: H256::repeat_byte(1),
            last_period_dag_anchor_hash: H256::repeat_byte(2),
            dag_genesis_hash: H256::repeat_byte(2),
            dag_blocks_size: 10,
            ghost_path_move_back: 0,
            pbft_gas_limit: 100,
            extra_data_required: false,
            extra_data_available: false,
            final_chain_hash_valid: true,
            final_chain_hash: H256::repeat_byte(3),
            wallets: Vec::new(),
            ghost_path: Vec::new(),
            has_non_finalized_fallback: false,
            non_finalized_fallback_hash: H256::zero(),
        };

        service.begin_runtime_session(tick);
        service.begin_proposal_session(proposal.clone());
        service.begin_pbft_sync_queue_drain();
        assert!(service.runtime_session_next().is_none());
        assert!(service.proposal_session_next().is_none());
        assert!(service.pbft_sync_queue_drain_next().is_none());

        service.begin_state_action_effect_session(PbftManagerStateActionFact {
            state: PbftManagerRuntimeStateCode::Filter,
            period: 10,
            round: 2,
            step: 3,
            elapsed_round_ms: 250,
            deadline_ms: 1_000,
            current_round_lambda_ms: 100,
            polling_interval_ms: 100,
            has_previous_round_next_null: false,
            has_previous_round_next_value: false,
            previous_round_next_value_hash: [0x11; 32],
            has_current_round_soft_value: false,
            current_round_soft_value_hash: [0x22; 32],
            has_cert_voted_block: false,
            cert_voted_block_hash: [0x33; 32],
            already_next_voted_value: false,
            already_next_voted_null: false,
        });
        assert!(service.state_action_effect_session_next().is_some());

        service.complete_bootstrap();
        service.begin_runtime_session(tick);
        assert!(service.runtime_session_next().is_some());
        service.abort_runtime_session();
        assert_eq!(
            service.runtime_session_next().unwrap().status,
            PbftManagerRuntimeStatus::ContractError
        );

        service.begin_proposal_session(proposal);
        assert!(service.proposal_session_next().is_some());
        service.begin_pbft_sync_queue_drain();
        assert!(service.pbft_sync_queue_drain_next().is_some());

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn pbft_chain_task_methods_preserve_chain_semantics() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_chain_task_wrapper");
        let service = PbftService::restore(storage, config(1)).unwrap();

        let initial = service.pbft_chain_head();
        assert_eq!(initial.size, 0);
        assert_eq!(initial.non_empty_size, 0);
        assert_eq!(initial.last_pbft_block_hash, H256::zero());
        assert_eq!(initial.last_non_null_pbft_dag_anchor_hash, H256::zero());

        assert!(matches!(
            service.pbft_chain_validate_block(1, H256::zero()),
            PbftBlockValidation::Valid
        ));

        assert!(
            !service
                .pbft_chain_block_exists(H256::from_low_u64_be(7))
                .unwrap()
        );
        let chain_update = service
            .pbft_chain_update(H256::from_low_u64_be(11), H256::zero())
            .unwrap();
        assert_eq!(chain_update.size, 1);
        assert_eq!(chain_update.last_pbft_block_hash, H256::from_low_u64_be(11));

        assert_eq!(
            service.pbft_chain_head(),
            PbftChainHead {
                head_hash: H256::zero(),
                size: 1,
                non_empty_size: 0,
                last_pbft_block_hash: H256::from_low_u64_be(11),
                last_non_null_pbft_dag_anchor_hash: H256::zero(),
            }
        );
        assert!(matches!(
            service.pbft_chain_validate_block(2, H256::from_low_u64_be(11)),
            PbftBlockValidation::Valid
        ));
        assert!(matches!(
            service.pbft_chain_validate_block(3, H256::from_low_u64_be(11)),
            PbftBlockValidation::PeriodMismatch {
                expected: 2,
                actual: 3
            }
        ));

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    fn service_finalization_intent(
        block_hash: H256,
        block_period: u64,
        block_prev_hash: H256,
        anchor_hash: H256,
    ) -> PbftFinalizationIntent {
        PbftFinalizationIntent {
            block_hash,
            block_period,
            block_prev_hash,
            pivot_dag_anchor_hash: anchor_hash,
            has_pillar_block: false,
            pillar_block_finalized: false,
            request_dynamic_lambda_update: false,
            cert_vote_count: 1,
            sample_cert_vote_block_hash: block_hash,
            sample_cert_vote_period: block_period,
            sample_cert_vote_round: 1,
            sample_cert_vote_step: 1,
            block_lambda: 1_500,
            last_saved_period_lambda_found: false,
            last_saved_period_lambda: 0,
            dynamic_blocks_per_year: 1_000,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 0,
            dpos_blocks_per_year: 500,
            period_data_rlp: vec![0xc0],
            ordered_dag_block_hashes: vec![H256::repeat_byte(1)],
            ordered_transaction_hashes: vec![H256::repeat_byte(2)],
            process_pillar_block_after_advance: false,
        }
    }

    #[test]
    fn plan_finalization_intent_derives_live_chain_state_and_head_payload() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_finalize_intent");
        let service = PbftService::restore(storage, config(1)).unwrap();

        let block_hash = H256::repeat_byte(0x11);
        let block_period = 1;
        let block_prev_hash = H256::zero();
        let anchor_hash = H256::repeat_byte(0x22);

        let plan = service
            .plan_finalization_intent(service_finalization_intent(
                block_hash,
                block_period,
                block_prev_hash,
                anchor_hash,
            ))
            .unwrap();
        assert_eq!(plan.status, PbftFinalizationStatus::Accepted);
        assert_eq!(
            plan.storage_write_intent.pbft_head_hash,
            service.pbft_chain_head().head_hash
        );
        assert_eq!(plan.storage_write_intent.pbft_block_hash, block_hash);

        let expected_payload = service
            .chain()
            .finalization_snapshot(block_hash, Some(true))
            .unwrap()
            .1;
        assert_eq!(
            plan.storage_write_intent.pbft_head_payload,
            expected_payload
        );

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn plan_finalization_intent_rejects_stale_previous_hash_mismatch_for_non_advance() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_finalize_intent_stale");
        let service = PbftService::restore(storage, config(1)).unwrap();
        service
            .pbft_chain_update(H256::repeat_byte(0x12), H256::zero())
            .unwrap();
        service
            .pbft_chain_update(H256::repeat_byte(0x13), H256::zero())
            .unwrap();

        let plan = service
            .plan_finalization_intent(service_finalization_intent(
                H256::repeat_byte(0x14),
                2,
                H256::repeat_byte(0x55),
                H256::repeat_byte(0x33),
            ))
            .unwrap();
        assert_eq!(plan.status, PbftFinalizationStatus::StalePeriod);

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn plan_finalization_intent_rejects_previous_hash_mismatch_on_advance() {
        let (path, storage) =
            temp_storage("rustaxa_consensus_pbft_service_finalize_intent_prev_mismatch");
        let service = PbftService::restore(storage, config(1)).unwrap();
        service
            .pbft_chain_update(H256::repeat_byte(0x12), H256::zero())
            .unwrap();
        service
            .pbft_chain_update(H256::repeat_byte(0x13), H256::zero())
            .unwrap();

        let plan = service
            .plan_finalization_intent(service_finalization_intent(
                H256::repeat_byte(0x14),
                3,
                H256::repeat_byte(0x55),
                H256::repeat_byte(0x33),
            ))
            .unwrap();
        assert_eq!(plan.status, PbftFinalizationStatus::PreviousHashMismatch);

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn plan_finalization_intent_reports_poisoned_chain_lock() {
        let (path, storage) =
            temp_storage("rustaxa_consensus_pbft_service_finalize_intent_poisoned_chain");
        let service = PbftService::restore(storage, config(1)).unwrap();
        let chain = service.chain().clone();
        let poison = thread::spawn(move || {
            let _guard = chain.write().unwrap();
            panic!("poison PBFT chain lock");
        });
        assert!(poison.join().is_err());

        let error = service
            .plan_finalization_intent(service_finalization_intent(
                H256::repeat_byte(0x11),
                1,
                H256::zero(),
                H256::repeat_byte(0x22),
            ))
            .expect_err("poisoned chain lock should fail chain-derived planning");
        assert_eq!(error.to_string(), "PBFT_CHAIN_SERVICE_LOCK_POISONED");

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    fn composed_block_validation_fact(
        period: u64,
        pivot_hash: H256,
    ) -> crate::pbft_manager::PbftManagerBlockValidationFact {
        use crate::pbft_manager::PbftManagerBlockValidationFactStatus as FactStatus;

        crate::pbft_manager::PbftManagerBlockValidationFact {
            block_hash: H256::repeat_byte(0x31),
            period,
            pivot_hash,
            pivot_is_null: false,
            dag_order_required: true,
            extra_data_required: false,
            extra_data_present: false,
            extra_data_pillar_hash_present: false,
            pillar_block_required: false,
            pbft_chain_status: FactStatus::NotChecked,
            final_chain_hash_status: FactStatus::NotChecked,
            reward_votes_status: FactStatus::NotChecked,
            pillar_block_status: FactStatus::NotRequired,
            dag_order_status: FactStatus::NotChecked,
            dag_weight_status: FactStatus::Valid,
        }
    }

    fn composed_block_validation_candidate(
        fact: crate::pbft_manager::PbftManagerBlockValidationFact,
        previous_pbft_block_hash: H256,
        candidate_final_chain_hash: H256,
        expected_order_hash: H256,
        pbft_gas_limit: u64,
        reward_vote_hashes: Vec<H256>,
        pillar_block_hash: Option<H256>,
    ) -> PbftBlockValidationCandidate {
        PbftBlockValidationCandidate {
            fact,
            previous_pbft_block_hash,
            candidate_final_chain_hash,
            expected_order_hash,
            pbft_gas_limit,
            reward_vote_hashes,
            pillar_block_hash,
        }
    }

    #[test]
    fn composed_block_validation_composes_dag_and_terminal_accept() -> Result<()> {
        use crate::pbft_manager::{
            PbftManagerBlockValidationAction as Action, PbftManagerBlockValidationStatus as Status,
        };

        let (path, storage) = temp_storage("rustaxa_consensus_composed_block_valid");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let dag = dag_service(storage.clone());
        let final_chain = final_chain_with_pillar_voters_and_delay(storage.clone(), &[], 2);
        let genesis = H256::repeat_byte(1);
        let block_rlp = candidate_dag_block_rlp(genesis, 42);
        let block = dag_manager_block_from_rlp(&block_rlp)?;
        dag.lock_dag()?.state.add_block(block.clone())?;
        save_dag_block_to_storage(storage.as_ref(), block.hash, 1, 0, &block_rlp)?;
        let expected_order_hash = pbft_candidate_dag_order_hash(
            &dag.prepare_pbft_candidate_payload(1, block.hash)?
                .expect("payload should load")
                .payload,
        );

        let plan = service
            .validate_pbft_block_composed(
                &final_chain,
                &dag,
                composed_block_validation_candidate(
                    composed_block_validation_fact(1, block.hash),
                    H256::zero(),
                    H256::zero(),
                    expected_order_hash,
                    u64::MAX,
                    Vec::new(),
                    None,
                ),
            )
            .unwrap();
        assert_eq!(plan.action, Action::Accept);
        assert_eq!(plan.status, Status::Accepted);

        drop(service);
        let _ = fs::remove_dir_all(path);
        Ok(())
    }

    #[test]
    fn proposed_block_admission_composes_validation_and_marks_cache_valid() -> Result<()> {
        let (path, storage) = temp_storage("rustaxa_consensus_proposed_block_admission_valid");
        let service = PbftService::restore(storage.clone(), config(1))?;
        let dag = dag_service(storage.clone());
        let final_chain = final_chain_with_pillar_voters_and_delay(storage.clone(), &[], 2);
        let genesis = H256::repeat_byte(1);
        let dag_block_rlp = candidate_dag_block_rlp(genesis, 42);
        let dag_block = dag_manager_block_from_rlp(&dag_block_rlp)?;
        dag.lock_dag()?.state.add_block(dag_block.clone())?;
        save_dag_block_to_storage(storage.as_ref(), dag_block.hash, 1, 0, &dag_block_rlp)?;
        let order_hash = pbft_candidate_dag_order_hash(
            &dag.prepare_pbft_candidate_payload(1, dag_block.hash)?
                .expect("payload should load")
                .payload,
        );
        let (block_rlp, link) = proposed_admission_block_rlp(1, dag_block.hash, order_hash);
        service.publish_proposed_block(
            link.period,
            link.block_hash,
            link.pivot_dag_block_hash,
            block_rlp.clone(),
        )?;

        let result = service.admit_proposed_block(
            &final_chain,
            &dag,
            PbftProposedBlockAdmissionRequest {
                period: link.period,
                block_hash: link.block_hash,
                pbft_gas_limit: u64::MAX,
                extra_data_required: false,
                pillar_block_required: false,
            },
        )?;

        assert_eq!(
            result.status,
            PbftProposedBlockAdmissionStatus::AcceptedNewlyValidated
        );
        assert_eq!(result.block_rlp, block_rlp);
        assert!(
            service
                .proposed_block(link.period, link.block_hash)
                .expect("proposal remains indexed")
                .is_valid
        );

        drop(final_chain);
        drop(service);
        drop(dag);
        let _ = fs::remove_dir_all(path);
        Ok(())
    }

    #[test]
    fn local_proposal_selection_composes_vote_block_and_ranking() -> Result<()> {
        let (path, storage) = temp_storage("rustaxa_consensus_local_proposal_selection");
        let service = PbftService::restore(storage.clone(), config(1))?;
        let dag = dag_service(storage.clone());
        let final_chain = final_chain_with_vote_validator_and_delay(
            storage.clone(),
            voter_from_secret(&NODE_SECRET),
            vrf::public_key_from_secret(&VRF_SECRET)?,
            20_000,
            2,
        );
        let genesis = H256::repeat_byte(1);
        let dag_block_rlp = candidate_dag_block_rlp(genesis, 42);
        let dag_block = dag_manager_block_from_rlp(&dag_block_rlp)?;
        dag.lock_dag()?.state.add_block(dag_block.clone())?;
        save_dag_block_to_storage(storage.as_ref(), dag_block.hash, 1, 0, &dag_block_rlp)?;
        let order_hash = pbft_candidate_dag_order_hash(
            &dag.prepare_pbft_candidate_payload(1, dag_block.hash)?
                .expect("payload should load")
                .payload,
        );
        let (block_rlp, link) = proposed_admission_block_rlp(1, dag_block.hash, order_hash);
        let vote = service.generate_signed_vote_with_weight(
            &final_chain,
            PbftVoteGenerationInput {
                block_hash: link.block_hash,
                vote_type: PbftVoteType::Propose,
                period: 1,
                round: 2,
                step: 1,
                node_secret: NODE_SECRET,
                vrf_secret: VRF_SECRET,
                expected_voter: voter_from_secret(&NODE_SECRET).into(),
                expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET)?,
            },
            service.committee_size,
            service.number_of_proposers,
        )?;
        assert!(vote.has_weight && vote.weight > 0);
        let (vote_validation, _) = service.validate_verified_vote_with_final_chain_internal(
            &final_chain,
            &vote.vote_rlp,
            PbftVoteAdmissionValidationRequest {
                strict_vrf: true,
                committee_size: service.committee_size,
                number_of_proposers: service.number_of_proposers,
                has_preverified_weight: false,
                preverified_weight: 0,
            },
            false,
        )?;
        assert!(vote_validation.accepted, "{vote_validation:?}");

        let mut mismatched_weight = RlpStream::new_list(4);
        let weighted_vote = Rlp::new(&vote.vote_rlp);
        for index in 0..3 {
            mismatched_weight.append_raw(weighted_vote.at(index)?.as_raw(), 1);
        }
        mismatched_weight.append(&vote.weight.saturating_add(1));
        let mismatched_weight = mismatched_weight.out().to_vec();
        let unweighted_vote = build_slashing_pbft_vote_payload(&vote.vote_rlp)?.vote_rlp;

        let result = service.select_local_proposal_candidate(
            &final_chain,
            &dag,
            PbftLocalProposalSelectionRequest {
                candidates: vec![PbftLocalProposalCandidate {
                    block_rlp: block_rlp.clone(),
                    vote_rlp: vote.vote_rlp.clone(),
                }],
                period: 1,
                round: 2,
                pbft_gas_limit: u64::MAX,
                extra_data_required: false,
                pillar_block_required: false,
            },
        )?;
        assert!(result.selected, "{result:?}");
        assert_eq!(result.selected_index, 0);
        assert!(service.proposed_block(1, link.block_hash).is_none());

        let ineligible_vote = generate_pbft_vote(PbftVoteGenerationInput {
            block_hash: link.block_hash,
            vote_type: PbftVoteType::Propose,
            period: 1,
            round: 2,
            step: 1,
            node_secret: NODE_SECRET_TWO,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&NODE_SECRET_TWO).into(),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET)?,
        })?;
        let mut weighted_ineligible = RlpStream::new_list(4);
        let ineligible_rlp = Rlp::new(&ineligible_vote.vote_rlp);
        for index in 0..3 {
            weighted_ineligible.append_raw(ineligible_rlp.at(index)?.as_raw(), 1);
        }
        weighted_ineligible.append(&1_u64);
        let after_ineligible = service.select_local_proposal_candidate(
            &final_chain,
            &dag,
            PbftLocalProposalSelectionRequest {
                candidates: vec![
                    PbftLocalProposalCandidate {
                        block_rlp: block_rlp.clone(),
                        vote_rlp: weighted_ineligible.out().to_vec(),
                    },
                    PbftLocalProposalCandidate {
                        block_rlp: block_rlp.clone(),
                        vote_rlp: vote.vote_rlp.clone(),
                    },
                ],
                period: 1,
                round: 2,
                pbft_gas_limit: u64::MAX,
                extra_data_required: false,
                pillar_block_required: false,
            },
        )?;
        assert!(after_ineligible.selected, "{after_ineligible:?}");
        assert_eq!(after_ineligible.selected_index, 1);

        for invalid_vote in [mismatched_weight, unweighted_vote] {
            let error = service
                .select_local_proposal_candidate(
                    &final_chain,
                    &dag,
                    PbftLocalProposalSelectionRequest {
                        candidates: vec![PbftLocalProposalCandidate {
                            block_rlp: block_rlp.clone(),
                            vote_rlp: invalid_vote,
                        }],
                        period: 1,
                        round: 2,
                        pbft_gas_limit: u64::MAX,
                        extra_data_required: false,
                        pillar_block_required: false,
                    },
                )
                .expect_err("missing or mismatched embedded weight must fail closed");
            assert!(
                error
                    .to_string()
                    .contains("PBFT_LOCAL_PROPOSAL_EMBEDDED_WEIGHT_MISMATCH")
            );
            assert!(service.proposed_block(1, link.block_hash).is_none());
        }

        drop(final_chain);
        drop(service);
        drop(dag);
        let _ = fs::remove_dir_all(path);
        Ok(())
    }

    #[test]
    fn local_proposal_selection_returns_typed_empty_result() -> Result<()> {
        let (path, storage) = temp_storage("rustaxa_consensus_local_proposal_empty");
        let service = PbftService::restore(storage.clone(), config(1))?;
        let dag = dag_service(storage.clone());
        let final_chain = final_chain_with_pillar_voters_and_delay(storage, &[], 2);
        let result = service.select_local_proposal_candidate(
            &final_chain,
            &dag,
            PbftLocalProposalSelectionRequest {
                candidates: Vec::new(),
                period: 1,
                round: 2,
                pbft_gas_limit: u64::MAX,
                extra_data_required: false,
                pillar_block_required: false,
            },
        )?;
        assert!(!result.selected);
        assert_eq!(result.selected_index, 0);
        assert_eq!(result.error_code, "PBFT_MANAGER_LEADER_EMPTY");

        drop(final_chain);
        drop(service);
        drop(dag);
        let _ = fs::remove_dir_all(path);
        Ok(())
    }

    #[test]
    fn composed_block_validation_rejects_dag_missing_order() {
        use crate::pbft_manager::{
            PbftManagerBlockValidationAction as Action, PbftManagerBlockValidationStatus as Status,
        };

        let (path, storage) = temp_storage("rustaxa_consensus_composed_block_dag_missing");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let dag = dag_service(storage.clone());
        let final_chain = final_chain_with_pillar_voters_and_delay(storage, &[], 2);

        let mut fact = composed_block_validation_fact(1, H256::repeat_byte(0x99));
        fact.pivot_is_null = true;
        fact.dag_order_required = false;
        let plan = service
            .validate_pbft_block_composed(
                &final_chain,
                &dag,
                composed_block_validation_candidate(
                    fact,
                    H256::zero(),
                    H256::zero(),
                    H256::zero(),
                    u64::MAX,
                    Vec::new(),
                    None,
                ),
            )
            .unwrap();

        assert_eq!(plan.action, Action::Reject);
        assert_eq!(plan.status, Status::DagOrderMissing);

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn composed_block_validation_rejects_dag_order_hash_invalid() -> Result<()> {
        use crate::pbft_manager::{
            PbftManagerBlockValidationAction as Action, PbftManagerBlockValidationStatus as Status,
        };

        let (path, storage) =
            temp_storage("rustaxa_consensus_composed_block_dag_order_hash_invalid");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let dag = dag_service(storage.clone());
        let final_chain = final_chain_with_pillar_voters_and_delay(storage.clone(), &[], 2);
        let genesis = H256::repeat_byte(1);
        let block_rlp = candidate_dag_block_rlp(genesis, 42);
        let block = dag_manager_block_from_rlp(&block_rlp)?;
        dag.lock_dag()?.state.add_block(block.clone())?;
        save_dag_block_to_storage(storage.as_ref(), block.hash, 1, 0, &block_rlp)?;
        let expected_hash = H256::repeat_byte(0xbb);

        let plan = service
            .validate_pbft_block_composed(
                &final_chain,
                &dag,
                composed_block_validation_candidate(
                    composed_block_validation_fact(1, block.hash),
                    H256::zero(),
                    H256::zero(),
                    expected_hash,
                    u64::MAX,
                    Vec::new(),
                    None,
                ),
            )
            .unwrap();

        assert_eq!(plan.action, Action::Reject);
        assert_eq!(plan.status, Status::DagOrderInvalid);

        drop(service);
        let _ = fs::remove_dir_all(path);
        Ok(())
    }

    #[test]
    fn composed_block_validation_rejects_dag_weight_exceeded() -> Result<()> {
        use crate::pbft_manager::{
            PbftManagerBlockValidationAction as Action, PbftManagerBlockValidationStatus as Status,
        };

        let (path, storage) = temp_storage("rustaxa_consensus_composed_block_dag_weight_invalid");
        let service = PbftService::restore(storage.clone(), config(1))?;
        let dag = dag_service(storage.clone());
        let genesis = H256::repeat_byte(1);
        let reward_block = H256::repeat_byte(0xa1);
        let reward_hashes = seed_reward_cert_votes(&service, reward_block, 1);
        service.apply_reward_votes_reset(RewardVoteResetApplyRequest {
            period: 1,
            round: 2,
            step: 3,
            block_hash: reward_block,
            sync: false,
        })?;

        dag.lock_dag()?.state.advance_empty_period(1)?;
        let first_rlp = candidate_dag_block_rlp(genesis, 42);
        let first = dag_manager_block_from_rlp(&first_rlp)?;
        dag.lock_dag()?.state.add_block(first.clone())?;
        save_dag_block_to_storage(storage.as_ref(), first.hash, 1, 0, &first_rlp)?;
        let second_rlp = candidate_dag_block_rlp(genesis, 43);
        let second = dag_manager_block_from_rlp(&second_rlp)?;
        dag.lock_dag()?.state.add_block(second.clone())?;
        save_dag_block_to_storage(storage.as_ref(), second.hash, 1, 0, &second_rlp)?;

        let ghost = dag.dag_ghost_path(crate::dag_transaction_service::DagGhostPathRoot::Block(
            genesis,
        ))?;
        let divergent = if ghost[1] == first.hash {
            second.hash
        } else {
            first.hash
        };
        let prepared = dag
            .prepare_pbft_candidate_payload(2, divergent)?
            .expect("divergent candidate payload");
        let order_hash = pbft_candidate_dag_order_hash(&prepared.payload);

        let (previous_rlp, previous) = pbft_block_rlp_with_pivot(H256::zero(), genesis, 1);
        storage
            .period()
            .write(1, &period_data_with_pbft_block(&previous_rlp))?;
        storage.period().write_pbft_period(previous.block_hash, 1)?;
        service.pbft_chain_update(previous.block_hash, genesis)?;

        let plan = service.validate_pbft_block_composed(
            &final_chain_with_pillar_voters_and_delay(storage.clone(), &[], 2),
            &dag,
            composed_block_validation_candidate(
                composed_block_validation_fact(2, divergent),
                previous.block_hash,
                H256::zero(),
                order_hash,
                41,
                reward_hashes,
                None,
            ),
        )?;
        assert_eq!(plan.action, Action::Reject);
        assert_eq!(plan.status, Status::DagWeightInvalid);
        assert!(
            !service
                .manager_state()
                .state
                .has_cached_anchor_dag_order(divergent)
        );

        drop(dag);
        drop(service);
        let _ = fs::remove_dir_all(path);
        Ok(())
    }

    #[test]
    fn composed_block_validation_rejects_invalid_pbft_chain() {
        use crate::pbft_manager::{
            PbftManagerBlockValidationAction as Action, PbftManagerBlockValidationStatus as Status,
        };

        let (path, storage) = temp_storage("rustaxa_consensus_composed_block_chain_invalid");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let final_chain = final_chain_with_pillar_voters_and_delay(storage.clone(), &[], 2);
        let mut fact = composed_block_validation_fact(1, H256::repeat_byte(0x32));
        fact.pbft_chain_status = crate::pbft_manager::PbftManagerBlockValidationFactStatus::Valid;
        fact.final_chain_hash_status =
            crate::pbft_manager::PbftManagerBlockValidationFactStatus::Valid;
        fact.reward_votes_status = crate::pbft_manager::PbftManagerBlockValidationFactStatus::Valid;
        let plan = service
            .validate_pbft_block_composed(
                &final_chain,
                &dag_service(storage.clone()),
                composed_block_validation_candidate(
                    fact,
                    H256::repeat_byte(0x41),
                    H256::zero(),
                    H256::zero(),
                    u64::MAX,
                    Vec::new(),
                    None,
                ),
            )
            .unwrap();

        assert_eq!(plan.action, Action::Reject);
        assert_eq!(plan.status, Status::PbftChainInvalid);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn composed_block_validation_rejects_invalid_final_chain_hash() {
        use crate::pbft_manager::{
            PbftManagerBlockValidationAction as Action, PbftManagerBlockValidationStatus as Status,
        };

        let (path, storage) = temp_storage("rustaxa_consensus_composed_block_final_chain_invalid");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let final_chain = final_chain_with_pillar_voters_and_delay(storage.clone(), &[], 2);
        let plan = service
            .validate_pbft_block_composed(
                &final_chain,
                &dag_service(storage.clone()),
                composed_block_validation_candidate(
                    composed_block_validation_fact(1, H256::repeat_byte(0x32)),
                    H256::zero(),
                    H256::repeat_byte(0x42),
                    H256::zero(),
                    u64::MAX,
                    Vec::new(),
                    None,
                ),
            )
            .unwrap();

        assert_eq!(plan.action, Action::Reject);
        assert_eq!(plan.status, Status::FinalChainHashInvalid);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn composed_block_validation_rejects_invalid_reward_votes() {
        use crate::pbft_manager::{
            PbftManagerBlockValidationAction as Action, PbftManagerBlockValidationStatus as Status,
        };

        let (path, storage) = temp_storage("rustaxa_consensus_composed_block_reward_invalid");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let previous = H256::repeat_byte(0x43);
        service.pbft_chain_update(previous, H256::zero()).unwrap();
        let final_chain = final_chain_with_pillar_voters_and_delay(storage.clone(), &[], 2);
        let plan = service
            .validate_pbft_block_composed(
                &final_chain,
                &dag_service(storage.clone()),
                composed_block_validation_candidate(
                    composed_block_validation_fact(2, H256::repeat_byte(0x32)),
                    previous,
                    H256::zero(),
                    H256::zero(),
                    u64::MAX,
                    vec![H256::repeat_byte(0x44)],
                    None,
                ),
            )
            .unwrap();

        assert_eq!(plan.action, Action::Reject);
        assert_eq!(plan.status, Status::RewardVotesInvalid);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn composed_block_validation_rejects_missing_pillar_anchor() {
        use crate::pbft_manager::{
            PbftManagerBlockValidationAction as Action,
            PbftManagerBlockValidationFactStatus as FactStatus,
            PbftManagerBlockValidationStatus as Status,
        };

        let (path, storage) = temp_storage("rustaxa_consensus_composed_block_pillar_invalid");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        service.complete_pillar_bootstrap().unwrap();
        let final_chain = final_chain_with_pillar_voters_and_delay(storage.clone(), &[], 2);
        let mut fact = composed_block_validation_fact(1, H256::repeat_byte(0x32));
        fact.extra_data_required = true;
        fact.extra_data_present = true;
        fact.extra_data_pillar_hash_present = true;
        fact.pillar_block_required = true;
        fact.pillar_block_status = FactStatus::NotChecked;
        let plan = service
            .validate_pbft_block_composed(
                &final_chain,
                &dag_service(storage.clone()),
                composed_block_validation_candidate(
                    fact,
                    H256::zero(),
                    H256::zero(),
                    H256::zero(),
                    u64::MAX,
                    Vec::new(),
                    Some(H256::repeat_byte(0x45)),
                ),
            )
            .unwrap();

        assert_eq!(plan.action, Action::Reject);
        assert_eq!(plan.status, Status::PillarBlockInvalid);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn composed_block_validation_returns_final_chain_wait_without_looping() {
        use crate::pbft_manager::{
            PbftManagerBlockValidationAction as Action,
            PbftManagerBlockValidationNextCheck as NextCheck,
            PbftManagerBlockValidationStatus as Status,
        };

        let (path, storage) = temp_storage("rustaxa_consensus_composed_block_final_chain_wait");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let final_chain = final_chain_with_pillar_voters(storage.clone(), &[]);
        let plan = service
            .validate_pbft_block_composed(
                &final_chain,
                &dag_service(storage.clone()),
                composed_block_validation_candidate(
                    composed_block_validation_fact(1, H256::repeat_byte(0x32)),
                    H256::zero(),
                    H256::zero(),
                    H256::zero(),
                    u64::MAX,
                    Vec::new(),
                    None,
                ),
            )
            .unwrap();

        assert_eq!(plan.action, Action::WaitForFinalization);
        assert_eq!(plan.status, Status::FinalChainHashMissing);
        assert_eq!(plan.next_check, NextCheck::ValidateFinalChainHash);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn composed_block_validation_propagates_pillar_readiness_error() {
        use crate::pbft_manager::PbftManagerBlockValidationFactStatus as FactStatus;

        let (path, storage) = temp_storage("rustaxa_consensus_composed_block_pillar_error");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let final_chain = final_chain_with_pillar_voters_and_delay(storage.clone(), &[], 2);
        let mut fact = composed_block_validation_fact(1, H256::repeat_byte(0x32));
        fact.extra_data_required = true;
        fact.extra_data_present = true;
        fact.extra_data_pillar_hash_present = true;
        fact.pillar_block_required = true;
        fact.pillar_block_status = FactStatus::NotChecked;
        let error = service
            .validate_pbft_block_composed(
                &final_chain,
                &dag_service(storage.clone()),
                composed_block_validation_candidate(
                    fact,
                    H256::zero(),
                    H256::zero(),
                    H256::zero(),
                    u64::MAX,
                    Vec::new(),
                    Some(H256::repeat_byte(0x46)),
                ),
            )
            .expect_err("pillar readiness failures propagate");

        assert!(
            error
                .to_string()
                .contains("PBFT_SERVICE_PILLAR_UNAVAILABLE"),
            "unexpected pillar readiness error: {error:#}"
        );
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn pbft_service_proposed_block_task_methods_preserve_semantics() {
        let (path, storage) =
            temp_storage("rustaxa_consensus_pbft_service_proposed_block_task_wrapper");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();

        let (rlp, link) = pbft_block_rlp(13, 12_345);
        assert!(
            service
                .publish_proposed_block(
                    link.period,
                    link.block_hash,
                    link.pivot_dag_block_hash,
                    rlp.clone()
                )
                .unwrap()
        );
        assert_eq!(storage.pbft().proposed_rlp().unwrap().len(), 1);
        let first = service
            .proposed_block(link.period, link.block_hash)
            .expect("proposal should be published");
        assert_eq!(first.block_hash, link.block_hash);
        assert_eq!(first.pivot_hash, link.pivot_dag_block_hash);
        assert!(!first.is_valid);

        service
            .mark_proposed_block_valid(link.period, link.block_hash)
            .unwrap();
        let marked = service
            .proposed_block(link.period, link.block_hash)
            .expect("proposal should still exist after mark");
        assert!(marked.is_valid);
        assert_eq!(service.proposed_blocks().snapshot_entries().len(), 1);
        assert!(service.proposed_blocks().snapshot_entries()[0].is_valid);
        assert!(
            service
                .proposed_block(link.period, H256::from_low_u64_be(99))
                .is_none()
        );

        assert!(
            !service
                .publish_proposed_block(
                    link.period,
                    link.block_hash,
                    link.pivot_dag_block_hash,
                    rlp
                )
                .unwrap()
        );
        assert_eq!(storage.pbft().proposed_rlp().unwrap().len(), 1);

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn sync_admission_is_owned_by_native_pbft_service() {
        use crate::pbft_sync::{
            PbftSyncAdmissionInitialFact, PbftSyncAdmissionTransactionReport, PbftSyncFactStatus,
            PbftSyncRuntimeFinalChainHashStatus,
        };

        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_sync_owner");
        storage
            .period()
            .write(9, &[0xc8, 0xc0, 0xc1])
            .expect("period data persists");
        let service = PbftService::restore(storage, config(1)).unwrap();
        let initial = PbftSyncAdmissionInitialFact {
            block_period: 10,
            block_prev_hash: H256::repeat_byte(9),
            chain_last_hash: H256::repeat_byte(9),
            chain_last_period: 9,
            block_in_chain: false,
            candidate_final_chain_hash: H256::zero(),
            reward_vote_hashes: Vec::new(),
            dag_transaction_hashes: vec![H256::repeat_byte(1)],
            period_data_transaction_hashes: Vec::new(),
            extra_data_required: false,
            extra_data_present: false,
            extra_data_pillar_block_hash_present: false,
            pillar_votes_required: false,
            pillar_votes_present: false,
            previous_cert_votes_present: true,
            previous_cert_first_vote_has_weight: false,
        };

        assert!(!service.begin_pbft_sync_admission(initial.clone()));
        assert!(service.pbft_sync_admission_next().is_none());
        service.complete_bootstrap();
        assert!(service.begin_pbft_sync_admission(initial.clone()));

        let final_chain = service.pbft_sync_admission_next().expect("session starts");
        let reward = service
            .report_pbft_sync_admission_status(
                final_chain.cursor,
                final_chain.next_check,
                PbftSyncRuntimeFinalChainHashStatus::Valid,
                PbftSyncFactStatus::Valid,
            )
            .expect("FinalChain report advances");
        let cert = service
            .report_pbft_sync_admission_status(
                reward.cursor,
                reward.next_check,
                PbftSyncRuntimeFinalChainHashStatus::Valid,
                PbftSyncFactStatus::Valid,
            )
            .expect("reward report advances");
        let transactions = service
            .report_pbft_sync_admission_status(
                cert.cursor,
                cert.next_check,
                PbftSyncRuntimeFinalChainHashStatus::Valid,
                PbftSyncFactStatus::Valid,
            )
            .expect("cert report advances");
        assert_eq!(
            transactions.next_check,
            crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::CheckTransactions
        );
        let accepted = report_sync_admission_transactions_for_test(
            &service,
            PbftSyncAdmissionTransactionReport {
                missing_transaction_hashes: vec![H256::repeat_byte(1)],
                finalized_transaction_hashes: vec![H256::repeat_byte(2)],
                contains_finalized_transactions: true,
            },
        );
        assert!(accepted.complete);
        assert!(accepted.plan.accept_period_data);
        assert_eq!(accepted.plan.warnings.len(), 2);
        assert!(service.pbft_sync_admission_next().is_none());

        assert!(service.begin_pbft_sync_admission(initial));
        let step = service
            .pbft_sync_admission_next()
            .expect("replacement starts");
        let mismatch = service
            .report_pbft_sync_admission_status(
                step.cursor + 1,
                step.next_check,
                PbftSyncRuntimeFinalChainHashStatus::Valid,
                PbftSyncFactStatus::Valid,
            )
            .expect("mismatch returns terminal step");
        assert!(mismatch.complete);
        assert!(!mismatch.can_continue);
        assert!(service.pbft_sync_admission_next().is_none());

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    fn sync_final_chain_admission_fact(
        candidate_final_chain_hash: H256,
        reward_vote_hashes: Vec<H256>,
    ) -> crate::pbft_sync::PbftSyncAdmissionInitialFact {
        let mut fact = sync_transaction_admission_fact(Vec::new(), Vec::new());
        fact.block_period = 2;
        fact.chain_last_period = 1;
        fact.candidate_final_chain_hash = candidate_final_chain_hash;
        fact.reward_vote_hashes = reward_vote_hashes;
        fact
    }

    #[test]
    fn sync_final_chain_hash_admission_reports_valid_expected_hash() {
        let (path, storage) = temp_storage("rustaxa_consensus_sync_final_chain_valid");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        service.complete_bootstrap();
        let final_chain = final_chain_with_pillar_voters_and_delay(storage, &[], 2);
        assert!(
            service.begin_pbft_sync_admission(sync_final_chain_admission_fact(
                H256::zero(),
                vec![H256::repeat_byte(0x81)],
            ))
        );

        let (_, records, validation) = service
            .validate_pbft_sync_admission_final_chain_hash(&final_chain)
            .expect("exact FinalChain request");
        assert!(records.is_empty());
        assert_eq!(
            validation.status,
            crate::pbft_manager::PbftManagerFinalChainHashStatus::Valid
        );
        assert_eq!(validation.expected_hash, H256::zero());
        assert!(validation.error_code.is_empty());
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn sync_final_chain_hash_admission_reports_invalid_expected_hash() {
        let (path, storage) = temp_storage("rustaxa_consensus_sync_final_chain_invalid");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        service.complete_bootstrap();
        let final_chain = final_chain_with_pillar_voters_and_delay(storage, &[], 2);
        assert!(
            service.begin_pbft_sync_admission(sync_final_chain_admission_fact(
                H256::repeat_byte(0x91),
                Vec::new(),
            ))
        );

        let (_, records, validation) = service
            .validate_pbft_sync_admission_final_chain_hash(&final_chain)
            .expect("exact FinalChain request");
        assert!(records.is_empty());
        assert_eq!(
            validation.status,
            crate::pbft_manager::PbftManagerFinalChainHashStatus::Invalid
        );
        assert_eq!(validation.expected_hash, H256::zero());
        assert_eq!(
            validation.error_code,
            "PBFT_MANAGER_FINAL_CHAIN_HASH_MISMATCH"
        );
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn sync_final_chain_hash_admission_reports_missing_hash() {
        let (path, storage) = temp_storage("rustaxa_consensus_sync_final_chain_missing");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        service.complete_bootstrap();
        let final_chain = final_chain_with_pillar_voters(storage, &[]);
        assert!(
            service.begin_pbft_sync_admission(sync_final_chain_admission_fact(
                H256::zero(),
                Vec::new(),
            ))
        );

        let (step, records, validation) = service
            .validate_pbft_sync_admission_final_chain_hash(&final_chain)
            .expect("exact FinalChain request");
        assert!(records.is_empty());
        assert_eq!(
            validation.status,
            crate::pbft_manager::PbftManagerFinalChainHashStatus::Missing
        );
        assert_eq!(
            validation.error_code,
            "PBFT_MANAGER_FINAL_CHAIN_HASH_MISSING"
        );
        assert!(step.plan.wait_for_finalization);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn stale_sync_final_chain_completion_preserves_replacement_reward_request() {
        let (path, storage) = temp_storage("rustaxa_consensus_sync_final_chain_stale");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        service.complete_bootstrap();
        let final_chain = final_chain_with_pillar_voters_and_delay(storage, &[], 2);
        assert!(
            service.begin_pbft_sync_admission(sync_final_chain_admission_fact(
                H256::zero(),
                vec![H256::repeat_byte(0xa1)],
            ))
        );
        let replacement =
            sync_final_chain_admission_fact(H256::repeat_byte(0xa2), vec![H256::repeat_byte(0xa3)]);

        assert!(
            service
                .validate_pbft_sync_admission_final_chain_hash_with(&final_chain, || {
                    assert!(service.begin_pbft_sync_admission(replacement));
                    let replacement_final_chain = service.pbft_sync_admission_next().unwrap();
                    let replacement_reward = service
                        .report_pbft_sync_admission_status(
                            replacement_final_chain.cursor,
                            replacement_final_chain.next_check,
                            crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus::Valid,
                            crate::pbft_sync::PbftSyncFactStatus::Valid,
                        )
                        .unwrap();
                    assert_eq!(
                        replacement_reward.next_check,
                        crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::CheckRewardVotes
                    );
                })
                .is_none()
        );
        let replacement_step = service.pbft_sync_admission_next().unwrap();
        assert_eq!(
            replacement_step.next_check,
            crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::CheckRewardVotes
        );
        assert_eq!(
            service
                .manager
                .pbft_sync_admission_reward_request()
                .unwrap()
                .reward_vote_hashes,
            vec![H256::repeat_byte(0xa3)]
        );
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn sync_reward_admission_derives_hashes_and_reports_native_rejection() {
        let (path, storage) = temp_storage("rustaxa_consensus_sync_reward_rejected");
        let service = PbftService::restore(storage, config(0)).unwrap();
        service.complete_bootstrap();
        let requested = vec![H256::repeat_byte(0x61), H256::repeat_byte(0x62)];
        let reward_step = advance_sync_admission_to_reward_votes(&service, requested.clone());
        let identity = service
            .manager
            .pbft_sync_admission_reward_request()
            .expect("reward request identity");
        assert_eq!(identity.cursor, reward_step.cursor);
        assert_eq!(identity.block_period, 2);
        assert_eq!(identity.reward_vote_hashes, requested);

        let (step, records) = service
            .validate_pbft_sync_admission_reward_votes()
            .expect("exact reward request remains pending");
        assert!(records.is_empty());
        assert!(step.complete);
        assert!(!step.plan.accept_period_data);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn atomic_predecessor_report_reward_capture_rejects_stale_completion() {
        let (path, storage) = temp_storage("rustaxa_consensus_sync_reward_stale");
        let service = PbftService::restore(storage, config(0)).unwrap();
        service.complete_bootstrap();
        let mut first = sync_transaction_admission_fact(Vec::new(), Vec::new());
        first.block_period = 2;
        first.chain_last_period = 1;
        first.reward_vote_hashes = vec![H256::repeat_byte(0x71)];
        assert!(service.begin_pbft_sync_admission(first));
        let first_final_chain = service.pbft_sync_admission_next().unwrap();

        let mut replacement = sync_transaction_admission_fact(Vec::new(), Vec::new());
        replacement.block_period = 2;
        replacement.chain_last_period = 1;
        replacement.reward_vote_hashes = vec![H256::repeat_byte(0x72)];

        assert!(
            service
                .report_pbft_sync_admission_status_with_reward_votes_with(
                    first_final_chain.cursor,
                    first_final_chain.next_check,
                    crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus::Valid,
                    crate::pbft_sync::PbftSyncFactStatus::Valid,
                    || {
                        assert!(service.begin_pbft_sync_admission(replacement));
                        let replacement_final_chain = service.pbft_sync_admission_next().unwrap();
                        let replacement_reward = service
                            .report_pbft_sync_admission_status(
                                replacement_final_chain.cursor,
                                replacement_final_chain.next_check,
                                crate::pbft_sync::PbftSyncRuntimeFinalChainHashStatus::Valid,
                                crate::pbft_sync::PbftSyncFactStatus::Valid,
                            )
                            .unwrap();
                        assert_eq!(
                            replacement_reward.next_check,
                            crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::CheckRewardVotes
                        );
                    },
                )
                .is_none()
        );
        let replacement_step = service.pbft_sync_admission_next().unwrap();
        assert_eq!(
            replacement_step.next_check,
            crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::CheckRewardVotes
        );
        let replacement_identity = service
            .manager
            .pbft_sync_admission_reward_request()
            .unwrap();
        assert_eq!(
            replacement_identity.reward_vote_hashes,
            vec![H256::repeat_byte(0x72)]
        );
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn sync_transaction_admission_accepts_valid_native_inputs() {
        let (path, storage) = temp_storage("rustaxa_consensus_sync_transaction_valid");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        service.complete_bootstrap();
        let dag_transaction_service = dag_service(storage.clone());
        let final_chain = final_chain_with_pillar_voters(storage, &[]);
        let hash = H256::repeat_byte(0x11);
        advance_sync_admission_to_transactions(
            &service,
            sync_transaction_admission_fact(vec![hash], vec![hash]),
        );

        let step = service
            .validate_pbft_sync_admission_transactions(
                &dag_transaction_service,
                &final_chain,
                vec![PeriodDataQueueTransactionIdentity {
                    input_index: 7,
                    hash,
                    transaction_nonce: U256::one().to_big_endian(),
                    sender: [0x51; 20],
                }],
            )
            .expect("exact request remains pending");

        assert!(step.complete);
        assert!(step.plan.accept_period_data);
        assert!(step.plan.warnings.is_empty());
        assert!(!step.plan.contains_finalized_transaction_warning);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn sync_transaction_admission_preserves_missing_warning() {
        let (path, storage) = temp_storage("rustaxa_consensus_sync_transaction_missing");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        service.complete_bootstrap();
        let dag_transaction_service = dag_service(storage.clone());
        let final_chain = final_chain_with_pillar_voters(storage, &[]);
        let missing = H256::repeat_byte(0x22);
        advance_sync_admission_to_transactions(
            &service,
            sync_transaction_admission_fact(vec![missing], Vec::new()),
        );

        let step = service
            .validate_pbft_sync_admission_transactions(
                &dag_transaction_service,
                &final_chain,
                Vec::new(),
            )
            .unwrap();
        assert_eq!(
            step.plan.warnings,
            vec![crate::pbft_sync::PbftSyncTransactionWarning {
                hash: missing,
                kind: crate::pbft_sync::PbftSyncTransactionWarningKind::MissingTransaction,
            }]
        );
        assert!(!step.plan.contains_finalized_transaction_warning);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn sync_transaction_admission_preserves_first_finalized_warning() {
        let (path, storage) = temp_storage("rustaxa_consensus_sync_transaction_finalized");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        service.complete_bootstrap();
        let dag_transaction_service = dag_service(storage.clone());
        let final_chain = final_chain_with_pillar_voters(storage.clone(), &[]);
        let finalized = H256::repeat_byte(0x33);
        storage
            .transaction()
            .write_location(finalized, 1, 0, false)
            .unwrap();
        advance_sync_admission_to_transactions(
            &service,
            sync_transaction_admission_fact(vec![finalized], vec![finalized]),
        );

        let step = service
            .validate_pbft_sync_admission_transactions(
                &dag_transaction_service,
                &final_chain,
                vec![PeriodDataQueueTransactionIdentity {
                    input_index: 4,
                    hash: finalized,
                    transaction_nonce: [0; 32],
                    sender: [0x52; 20],
                }],
            )
            .unwrap();
        assert_eq!(
            step.plan.warnings,
            vec![crate::pbft_sync::PbftSyncTransactionWarning {
                hash: finalized,
                kind: crate::pbft_sync::PbftSyncTransactionWarningKind::FinalizedTransaction,
            }]
        );
        assert!(step.plan.contains_finalized_transaction_warning);
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn sync_transaction_admission_exact_aborts_native_lookup_error() {
        let (path, storage) = temp_storage("rustaxa_consensus_sync_transaction_error");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        service.complete_bootstrap();
        let dag_transaction_service = dag_service(storage.clone());
        let final_chain = final_chain_with_pillar_voters(storage, &[]);
        advance_sync_admission_to_transactions(
            &service,
            sync_transaction_admission_fact(vec![H256::zero()], Vec::new()),
        );

        let step = service
            .validate_pbft_sync_admission_transactions(
                &dag_transaction_service,
                &final_chain,
                Vec::new(),
            )
            .expect("zero lookup hash must return an exact terminal step");
        assert!(!step.can_continue);
        assert_eq!(step.error_code, "PBFT_SYNC_ADMISSION_SESSION_ABORTED");
        assert!(service.pbft_sync_admission_next().is_none());
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn stale_sync_transaction_error_does_not_abort_replacement_generation() {
        let (path, storage) = temp_storage("rustaxa_consensus_sync_transaction_stale_error");
        let service = PbftService::restore(storage.clone(), config(0)).unwrap();
        service.complete_bootstrap();
        let dag_transaction_service = dag_service(storage.clone());
        let final_chain = final_chain_with_pillar_voters(storage, &[]);
        advance_sync_admission_to_transactions(
            &service,
            sync_transaction_admission_fact(vec![H256::zero()], Vec::new()),
        );
        let replacement =
            sync_transaction_admission_fact(vec![H256::repeat_byte(0x42)], Vec::new());

        let stale = service.validate_pbft_sync_admission_transactions_with(
            &dag_transaction_service,
            &final_chain,
            Vec::new(),
            || assert!(service.begin_pbft_sync_admission(replacement)),
        );
        assert!(stale.is_none());
        let replacement_step = service.pbft_sync_admission_next().unwrap();
        assert_eq!(replacement_step.cursor, 0);
        assert_eq!(
            replacement_step.next_check,
            crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::ValidateFinalChainHash
        );
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn stale_sync_transaction_completion_does_not_mutate_replacement_generation() {
        let (path, storage) = temp_storage("rustaxa_consensus_sync_transaction_stale");
        let service = PbftService::restore(storage, config(0)).unwrap();
        service.complete_bootstrap();
        let first = sync_transaction_admission_fact(vec![H256::repeat_byte(0x41)], Vec::new());
        advance_sync_admission_to_transactions(&service, first);
        let stale = service
            .manager
            .pbft_sync_admission_transaction_request()
            .expect("transaction request identity");
        let replacement =
            sync_transaction_admission_fact(vec![H256::repeat_byte(0x42)], Vec::new());
        assert!(service.begin_pbft_sync_admission(replacement));

        assert!(
            service
                .manager
                .report_pbft_sync_admission_transactions_exact(
                    stale,
                    crate::pbft_sync::PbftSyncAdmissionTransactionReport {
                        missing_transaction_hashes: vec![H256::repeat_byte(0x41)],
                        finalized_transaction_hashes: Vec::new(),
                        contains_finalized_transactions: false,
                    },
                )
                .is_none()
        );
        let replacement_step = service.pbft_sync_admission_next().unwrap();
        assert_eq!(replacement_step.cursor, 0);
        assert_eq!(
            replacement_step.next_check,
            crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::ValidateFinalChainHash
        );
        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn get_pbft_sync_egress_is_owned_by_native_network_service() {
        use crate::network_api::{
            NETWORK_EFFECT_KIND_CLEAR_PEER_SYNCING, NETWORK_EFFECT_KIND_SEND_PACKET,
            NETWORK_PACKET_KIND_PBFT_BLOCKS_BUNDLE, NETWORK_PACKET_KIND_PBFT_SYNC,
            NetworkGetPbftSyncRequest,
        };

        let (path, storage) = temp_storage("rustaxa_consensus_pbft_sync_native_egress");
        storage.period().write(1, &[0xc1, 0x01]).unwrap();
        storage.period().write(2, &[0xc1, 0x02]).unwrap();
        let service = PbftService::restore(storage, config(1)).unwrap();
        service
            .chain()
            .update(H256::from_low_u64_be(1), H256::zero())
            .unwrap();
        service
            .chain()
            .update(H256::from_low_u64_be(2), H256::zero())
            .unwrap();
        let (proposal_rlp, proposal) = pbft_block_rlp(3, 30);
        service
            .publish_proposed_block(
                proposal.period,
                proposal.block_hash,
                proposal.pivot_dag_block_hash,
                proposal_rlp,
            )
            .unwrap();

        let mut request_rlp = rlp::RlpStream::new_list(1);
        request_rlp.append(&1u64);
        let network = service.network_service();
        let decision = network
            .ingest_get_pbft_sync_request(NetworkGetPbftSyncRequest {
                tarcap_version: 6,
                peer_id: [7; 64],
                request_rlp: request_rlp.out().to_vec(),
                source_payload_id: 55,
            })
            .unwrap();
        assert_eq!(decision.status, 0);
        assert_eq!(decision.queued_effect_count, 4);
        let effects = network.drain_work(6, 10).unwrap().effects;
        assert_eq!(effects.len(), 4);
        assert!(effects[..2].iter().all(|effect| {
            effect.kind == NETWORK_EFFECT_KIND_SEND_PACKET
                && effect.packet_kind == NETWORK_PACKET_KIND_PBFT_SYNC
        }));
        assert_eq!(effects[2].kind, NETWORK_EFFECT_KIND_CLEAR_PEER_SYNCING);
        assert_eq!(effects[2].dependency_id, 0);
        assert_eq!(
            effects[3].packet_kind,
            NETWORK_PACKET_KIND_PBFT_BLOCKS_BUNDLE
        );
        let final_sync = rlp::Rlp::new(&effects[1].payload_bytes);
        assert!(final_sync.val_at::<bool>(0).unwrap());
        assert_eq!(final_sync.at(1).unwrap().as_raw(), &[0xc1, 0x02]);

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn manager_and_public_chain_share_one_native_owner() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_shared_chain");
        let service = PbftService::restore(storage, config(1)).unwrap();

        service
            .chain()
            .update(
                ethereum_types::H256::from([7; 32]),
                ethereum_types::H256::from([4; 32]),
            )
            .unwrap();
        let public_head = service.chain().head();
        let manager_head = service
            .manager_state()
            .chain
            .read()
            .expect("PBFT chain lock should remain healthy")
            .state
            .head();
        assert_eq!(public_head, manager_head);

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn fresh_finalization_prepares_and_publishes_reward_votes_through_native_root() {
        use crate::pbft_finalize::PbftFinalizationRuntimeAction;

        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_reward_start");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let dag = dag_service(storage.clone());
        let block_hash = H256::repeat_byte(0x61);
        let _ = seed_reward_cert_votes(&service, block_hash, 12);

        let boundary = service
            .start_finalization_executor(&dag, reward_finalization_start_request(block_hash))
            .expect("native reward stage prepares and persists");
        assert_eq!(
            boundary.next_step.action,
            Some(PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime)
        );
        assert!(storage.extra_reward_votes_reset_generation() > 0);
        let durable = storage
            .pbft()
            .finalized_reward_vote_cursor()
            .unwrap()
            .expect("reward cursor persisted with primary storage");
        assert_eq!(durable.period, 12);
        assert_eq!(durable.round, 2);
        assert_eq!(durable.step, 3);
        assert_eq!(durable.block_hash, block_hash);
        assert!(!durable.votes_bundle_rlp.is_empty());

        let completed = service
            .advance_finalization_reward_votes_reset(boundary.next_step.action_index)
            .expect("native reward cursor publishes");
        assert!(completed.next_step.complete);
        let snapshot = service
            .verified_votes()
            .reward_vote_cursor_snapshot()
            .unwrap();
        assert!(snapshot.found);
        assert_eq!(snapshot.period, 12);
        assert_eq!(snapshot.round, 2);
        assert_eq!(snapshot.step, 3);
        assert_eq!(snapshot.block_hash, block_hash);

        drop(service);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn fresh_finalization_reward_identity_failure_clears_stale_manager_state() {
        use crate::pbft_finalize::{PbftFinalizationCleanupIntent, PbftFinalizationRuntimeAction};

        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_reward_start_reject");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let dag = dag_service(storage.clone());
        let _ = seed_reward_cert_votes(&service, H256::repeat_byte(0x61), 12);
        install_finalization_executor(
            &service,
            11,
            PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: false,
                reset_reward_votes: false,
                set_dag_block_order: false,
                update_sortition_params: false,
                update_finalized_transactions_status: false,
                update_pbft_chain: true,
                clear_anchor_dag_cache: false,
                finalize_final_chain: false,
                maybe_update_dynamic_lambda: false,
                advance_period: false,
                process_pillar_block: false,
            },
            vec![PbftFinalizationRuntimeAction::UpdatePbftChain],
        );
        {
            let mut manager = service.manager_state();
            manager.finalization_reward_votes_reset_generation = 99;
        }

        let error = service
            .start_finalization_executor(
                &dag,
                reward_finalization_start_request(H256::repeat_byte(0x62)),
            )
            .expect_err("mismatched reward identity rejects fresh start");
        assert!(
            error
                .to_string()
                .contains("PBFT_REWARD_VOTES_RESET_CERT_IDENTITY_MISMATCH")
        );
        assert_eq!(storage.extra_reward_votes_reset_generation(), 0);
        assert!(
            storage
                .pbft()
                .finalized_reward_vote_cursor()
                .unwrap()
                .is_none()
        );
        let manager = service.manager_state();
        assert!(manager.finalization_runtime_session.is_none());
        assert!(manager.finalization_runtime_plan.is_none());
        assert!(manager.finalization_sortition_commit_request.is_none());
        assert_eq!(manager.finalization_reward_votes_reset_generation, 0);
        drop(manager);

        drop(service);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn pillar_state_restarts_through_the_same_native_root() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_pillar_restart");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        service.complete_pillar_bootstrap().unwrap();
        let data = CurrentPillarBlockDataDb {
            pillar_block: PillarBlock {
                period: 1,
                state_root: H256::from_low_u64_be(1),
                previous_pillar_block_hash: H256::zero(),
                bridge_root: H256::from_low_u64_be(2),
                epoch: 3,
                validator_vote_count_changes: Vec::new(),
            },
            vote_counts: vec![ValidatorVoteCount {
                address: H160::from_low_u64_be(4),
                vote_count: 5,
            }],
        }
        .encode_rlp();
        service
            .apply_pillar_current_block_data_for_generation(data.clone(), 0)
            .unwrap();
        drop(service);

        let restarted = PbftService::restore(storage, config(1)).unwrap();
        assert!(!restarted.pillar_is_ready());
        assert_eq!(
            restarted
                .load_pillar_startup_bootstrap()
                .unwrap()
                .current_block_data_rlp,
            data
        );

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn pillar_final_chain_composition_is_owned_by_the_native_root() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_pillar_final_chain");
        let validator = [9; 20];
        let final_chain = final_chain_with_pillar_voters(storage.clone(), &[validator]);
        let service = PbftService::restore(storage, config(1)).unwrap();
        service.complete_pillar_bootstrap().unwrap();

        let threshold = service
            .pillar_consensus_threshold_with_final_chain(&final_chain, 0)
            .unwrap();
        assert!(threshold.available);
        assert!(threshold.threshold > 0);

        let plan = service
            .plan_pillar_block_creation_with_final_chain(
                &final_chain,
                PillarBlockCreationRequest {
                    pillar_block_period: 0,
                    state_root: H256::repeat_byte(1),
                    bridge_root: H256::repeat_byte(2),
                    bridge_epoch: H256::zero(),
                    first_pillar_block_period: 0,
                    pillar_blocks_interval: 10,
                },
            )
            .unwrap();
        assert!(plan.creation.valid);
        assert_eq!(plan.current_vote_counts.len(), 1);
        assert_eq!(plan.current_vote_counts[0].address, validator.into());
        assert!(plan.current_vote_counts[0].vote_count > 0);
        assert_eq!(plan.vote_count_changes.len(), 1);

        let replacement = CurrentPillarBlockDataDb {
            pillar_block: PillarBlock {
                period: 1,
                state_root: H256::repeat_byte(3),
                previous_pillar_block_hash: H256::zero(),
                bridge_root: H256::repeat_byte(4),
                epoch: 0,
                validator_vote_count_changes: Vec::new(),
            },
            vote_counts: Vec::new(),
        }
        .encode_rlp();
        service
            .apply_pillar_current_block_data_for_generation(
                replacement.clone(),
                plan.anchor_generation,
            )
            .unwrap();
        let stale = service
            .apply_pillar_current_block_data_for_generation(replacement, plan.anchor_generation)
            .unwrap_err();
        assert!(format!("{stale:#}").contains("PILLAR_BLOCK_CREATION_STALE_ANCHOR"));

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn pillar_vote_final_chain_composition_is_owned_by_the_native_root() {
        let context = PillarVoteSingleAdmissionContext {
            first_pillar_block_period: 0,
            pillar_blocks_interval: 10,
        };

        let (zero_path, zero_storage) =
            temp_storage("rustaxa_consensus_pbft_service_pillar_vote_zero");
        let zero_final_chain = final_chain_with_pillar_voters(zero_storage.clone(), &[]);
        let zero_service = PbftService::restore(zero_storage, config(1)).unwrap();
        zero_service.complete_pillar_bootstrap().unwrap();
        let (zero_anchor, zero_anchor_rlp) = pillar_current_data(0);
        zero_service
            .apply_pillar_current_block_data_for_generation(zero_anchor_rlp, 0)
            .unwrap();
        let (zero_vote, _) = signed_pillar_vote([0x71; 32], 1, zero_anchor.hash());
        let zero_plan = zero_service
            .validate_single_pillar_vote_with_final_chain(
                &zero_final_chain,
                zero_vote.encode_rlp(),
                context,
            )
            .unwrap();
        assert_eq!(zero_plan.status, 7);
        assert_eq!(zero_plan.vote_hash, zero_vote.hash(true).0);
        let missing = zero_service
            .pillar()
            .pbft_service_pillar_apply_prepared_single_vote_admission(
                crate::pillar_vote_service::PillarVoteSingleAdmissionApplyInput {
                    vote_hash: zero_vote.hash(true).0,
                    validator_vote_count: 5,
                    has_threshold: false,
                    threshold: 0,
                },
            )
            .unwrap();
        assert_eq!(missing.status, 11);
        drop(zero_service);
        let _ = fs::remove_dir_all(zero_path);

        let (apply_path, apply_storage) =
            temp_storage("rustaxa_consensus_pbft_service_pillar_vote_apply");
        let apply_secret = [0x72; 32];
        let apply_voter = voter_from_secret(&apply_secret);
        let apply_final_chain =
            final_chain_with_pillar_voters(apply_storage.clone(), &[apply_voter]);
        let apply_service = PbftService::restore(apply_storage, config(1)).unwrap();
        apply_service.complete_pillar_bootstrap().unwrap();
        let (apply_anchor, apply_anchor_rlp) = pillar_current_data(0);
        apply_service
            .apply_pillar_current_block_data_for_generation(apply_anchor_rlp, 0)
            .unwrap();
        let (apply_vote, _) = signed_pillar_vote(apply_secret, 1, apply_anchor.hash());
        let applied = apply_service
            .apply_single_pillar_vote_with_final_chain(
                &apply_final_chain,
                apply_vote.encode_rlp(),
                context,
                false,
            )
            .unwrap();
        assert_eq!(applied.status, 0);
        assert!(applied.accepted);
        assert!(applied.validator_vote_count > 0);
        assert_eq!(applied.voter, apply_voter);
        drop(apply_service);
        let _ = fs::remove_dir_all(apply_path);

        let (future_path, future_storage) =
            temp_storage("rustaxa_consensus_pbft_service_pillar_bundle_future");
        let future_secret = [0x73; 32];
        let future_voter = voter_from_secret(&future_secret);
        let future_final_chain =
            final_chain_with_pillar_voters(future_storage.clone(), &[future_voter]);
        let future_service = PbftService::restore(future_storage, config(1)).unwrap();
        future_service.complete_pillar_bootstrap().unwrap();
        let (future_anchor, future_anchor_rlp) = pillar_current_data(41);
        future_service
            .apply_pillar_current_block_data_for_generation(future_anchor_rlp, 0)
            .unwrap();
        let (future_vote, _) = signed_pillar_vote(future_secret, 42, future_anchor.hash());
        let future_plan = future_service
            .apply_pillar_vote_bundle_with_final_chain(
                &future_final_chain,
                vec![PillarVoteRlpPayload {
                    vote_rlp: future_vote.encode_rlp(),
                }],
                42,
            )
            .unwrap();
        assert!(future_plan.missing_threshold);
        drop(future_service);
        let _ = fs::remove_dir_all(future_path);

        let (bundle_path, bundle_storage) =
            temp_storage("rustaxa_consensus_pbft_service_pillar_bundle_zero");
        let bundle_final_chain = final_chain_with_pillar_voters(bundle_storage.clone(), &[]);
        let bundle_service = PbftService::restore(bundle_storage, config(1)).unwrap();
        bundle_service.complete_pillar_bootstrap().unwrap();
        let (bundle_anchor, bundle_anchor_rlp) = pillar_current_data(0);
        bundle_service
            .apply_pillar_current_block_data_for_generation(bundle_anchor_rlp, 0)
            .unwrap();
        let (bundle_vote, _) = signed_pillar_vote([0x74; 32], 1, bundle_anchor.hash());
        let bundle_hash: [u8; 32] = bundle_vote.hash(true).into();
        let bundle_plan = bundle_service
            .apply_pillar_vote_bundle_with_final_chain(
                &bundle_final_chain,
                vec![PillarVoteRlpPayload {
                    vote_rlp: bundle_vote.encode_rlp(),
                }],
                1,
            )
            .unwrap();
        assert_eq!(bundle_plan.status, 5);
        assert_eq!(bundle_plan.first_bad_vote_hash, bundle_hash);
        drop(bundle_service);
        let _ = fs::remove_dir_all(bundle_path);
    }

    #[test]
    fn sync_admission_native_pillar_bundle_accepts_valid_votes() -> Result<()> {
        let (path, storage) = temp_storage("rustaxa_consensus_sync_pillar_valid");
        let secret = [0xb1; 32];
        let voter = voter_from_secret(&secret);
        let final_chain = final_chain_with_pillar_voters(storage.clone(), &[voter]);
        let service = PbftService::restore(storage.clone(), config(1))?;
        service.complete_bootstrap();
        service.complete_pillar_bootstrap()?;
        let (anchor, anchor_rlp) = pillar_current_data(0);
        service.apply_pillar_current_block_data_for_generation(anchor_rlp, 0)?;
        let (vote, _) = signed_pillar_vote(secret, 1, anchor.hash());
        let pending = advance_sync_admission_to_pillar(&service, 1);
        assert_eq!(
            pending.next_check,
            crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::ValidatePillarVotes
        );

        let accepted = service
            .validate_pbft_sync_admission_pillar_votes(
                &final_chain,
                vec![PillarVoteRlpPayload {
                    vote_rlp: vote.encode_rlp(),
                }],
            )
            .expect("exact pillar report");
        assert_eq!(
            accepted.status,
            crate::pbft_sync::PbftSyncAdmissionSessionStatus::Accepted
        );
        assert!(accepted.plan.accept_period_data);

        drop(service);
        drop(final_chain);
        drop(storage);
        fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn sync_admission_native_pillar_bundle_maps_empty_and_unavailable_to_invalid() -> Result<()> {
        for (name, complete_pillar) in [("empty", true), ("unavailable", false)] {
            let (path, storage) = temp_storage(&format!("rustaxa_consensus_sync_pillar_{name}"));
            let final_chain = final_chain_with_pillar_voters(storage.clone(), &[]);
            let service = PbftService::restore(storage.clone(), config(1))?;
            service.complete_bootstrap();
            if complete_pillar {
                service.complete_pillar_bootstrap()?;
            }
            advance_sync_admission_to_pillar(&service, 1);

            let rejected = service
                .validate_pbft_sync_admission_pillar_votes(
                    &final_chain,
                    if complete_pillar {
                        Vec::new()
                    } else {
                        vec![PillarVoteRlpPayload {
                            vote_rlp: vec![0xc0],
                        }]
                    },
                )
                .expect("invalid pillar report");
            assert_eq!(
                rejected.status,
                crate::pbft_sync::PbftSyncAdmissionSessionStatus::FailedPeer
            );
            assert!(!rejected.plan.clear_sync_queue);
            assert!(rejected.plan.report_malicious_peer);

            drop(service);
            drop(final_chain);
            drop(storage);
            fs::remove_dir_all(path)?;
        }
        Ok(())
    }

    #[test]
    fn sync_admission_native_pillar_stale_generation_preserves_replacement() -> Result<()> {
        let (path, storage) = temp_storage("rustaxa_consensus_sync_pillar_stale");
        let service = PbftService::restore(storage.clone(), config(1))?;
        service.complete_bootstrap();
        advance_sync_admission_to_pillar(&service, 1);
        let stale_identity = service
            .manager
            .pbft_sync_admission_pillar_request()
            .expect("pillar request identity");

        assert!(service.begin_pbft_sync_admission(sync_pillar_admission_fact(2)));
        assert!(
            service
                .manager
                .report_pbft_sync_admission_pillar_status_exact(
                    stale_identity,
                    crate::pbft_sync::PbftSyncFactStatus::Valid,
                )
                .is_none()
        );
        let replacement = service
            .pbft_sync_admission_next()
            .expect("replacement remains active");
        assert_eq!(replacement.cursor, 0);
        assert_eq!(
            replacement.next_check,
            crate::pbft_sync::PbftSyncProcessRuntimeNextCheck::ValidateFinalChainHash
        );

        drop(service);
        drop(storage);
        fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn invalid_configuration_fails_before_root_publication() {
        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_failure");
        let mut invalid = config(1);
        invalid.genesis_lambda_ms = 0;

        let error = PbftService::restore(storage, invalid)
            .err()
            .expect("invalid immutable configuration must reject construction");
        assert!(
            error
                .to_string()
                .contains("PBFT_MANAGER_STARTUP_INVALID_LAMBDA_CONFIG")
        );

        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_advancement_error_clears_application_root_state() {
        use crate::pbft_finalize::{PbftFinalizationCleanupIntent, PbftFinalizationRuntimeAction};

        let (path, storage) =
            temp_storage("rustaxa_consensus_pbft_service_finalization_error_cleanup");
        let service = PbftService::restore(storage, config(1)).unwrap();
        install_finalization_executor(
            &service,
            1,
            PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: false,
                reset_reward_votes: true,
                set_dag_block_order: false,
                update_sortition_params: false,
                update_finalized_transactions_status: false,
                update_pbft_chain: false,
                clear_anchor_dag_cache: false,
                finalize_final_chain: false,
                maybe_update_dynamic_lambda: false,
                advance_period: false,
                process_pillar_block: false,
            },
            vec![PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime],
        );

        let error = service
            .advance_finalization_reward_votes_reset(0)
            .expect_err("missing reset generation must reject cursor publication");
        assert!(
            error
                .to_string()
                .contains("PBFT_FINALIZE_POST_STORAGE_REWARD_VOTES_INVARIANT")
        );
        let manager = service.manager_state();
        assert!(manager.finalization_runtime_session.is_none());
        assert!(manager.finalization_runtime_plan.is_none());
        assert!(manager.finalization_sortition_commit_request.is_none());
        assert_eq!(manager.finalization_reward_votes_reset_generation, 0);
        drop(manager);

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_dag_advancement_rejects_wrong_action_before_mutation() {
        use crate::pbft_finalize::{PbftFinalizationCleanupIntent, PbftFinalizationRuntimeAction};

        let (path, storage) =
            temp_storage("rustaxa_consensus_pbft_service_finalization_dag_wrong_action");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let dag = dag_service(storage);
        let initial_period = dag.lock_dag().unwrap().state.period();
        install_finalization_executor(
            &service,
            1,
            PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: false,
                reset_reward_votes: false,
                set_dag_block_order: false,
                update_sortition_params: false,
                update_finalized_transactions_status: false,
                update_pbft_chain: false,
                clear_anchor_dag_cache: false,
                finalize_final_chain: true,
                maybe_update_dynamic_lambda: false,
                advance_period: false,
                process_pillar_block: false,
            },
            vec![PbftFinalizationRuntimeAction::FinalizeFinalChain],
        );

        let boundary = service
            .advance_finalization_dag_order(&dag, 0)
            .expect("wrong action returns a terminal boundary");
        assert!(!boundary.refresh_dag_counters);
        assert!(boundary.expired_dag_hashes.is_empty());
        assert_eq!(dag.lock_dag().unwrap().state.period(), initial_period);
        assert!(
            service
                .manager_state()
                .finalization_runtime_session
                .is_none()
        );

        drop(service);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_dag_operational_error_clears_application_root_state() {
        use crate::pbft_finalize::{PbftFinalizationCleanupIntent, PbftFinalizationRuntimeAction};

        let (path, storage) = temp_storage("rustaxa_consensus_pbft_service_finalization_dag_error");
        let service = PbftService::restore(storage.clone(), config(1)).unwrap();
        let dag = dag_service(storage);
        install_finalization_executor(
            &service,
            1,
            PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: false,
                reset_reward_votes: false,
                set_dag_block_order: true,
                update_sortition_params: false,
                update_finalized_transactions_status: false,
                update_pbft_chain: false,
                clear_anchor_dag_cache: false,
                finalize_final_chain: false,
                maybe_update_dynamic_lambda: false,
                advance_period: false,
                process_pillar_block: false,
            },
            vec![PbftFinalizationRuntimeAction::SetDagBlockOrder],
        );

        let error = service
            .advance_finalization_dag_order(&dag, 0)
            .expect_err("missing retained anchor must reject native DAG application");
        assert!(
            error
                .to_string()
                .contains("DAG_RUNTIME_FINALIZATION_ANCHOR_BLOCK")
        );
        let manager = service.manager_state();
        assert!(manager.finalization_runtime_session.is_none());
        assert!(manager.finalization_runtime_plan.is_none());
        drop(manager);

        drop(service);
        drop(dag);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn finalization_period_and_pillar_advancement_share_native_boundary() {
        use crate::pbft_finalize::{
            PbftFinalizationCleanupIntent, PbftFinalizationRuntimeAction,
            PbftFinalizationRuntimeStatus,
        };

        let (path, storage) =
            temp_storage("rustaxa_consensus_pbft_service_finalization_period_pillar");
        let service = PbftService::restore(storage, config(1)).unwrap();
        service.manager_state().state.set_period_for_test(3);
        install_finalization_executor(
            &service,
            2,
            PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: false,
                reset_reward_votes: false,
                set_dag_block_order: false,
                update_sortition_params: false,
                update_finalized_transactions_status: false,
                update_pbft_chain: false,
                clear_anchor_dag_cache: false,
                finalize_final_chain: false,
                maybe_update_dynamic_lambda: false,
                advance_period: true,
                process_pillar_block: true,
            },
            vec![
                PbftFinalizationRuntimeAction::AdvancePeriod,
                PbftFinalizationRuntimeAction::ProcessPillarBlock,
            ],
        );

        let period = service
            .advance_finalization_advance_period(0)
            .expect("period advancement reaches pillar leaf");
        assert_eq!(
            period.next_step.action,
            Some(PbftFinalizationRuntimeAction::ProcessPillarBlock)
        );
        assert_eq!(period.snapshot.period, 3);
        assert!(
            service
                .manager_state()
                .finalization_runtime_session
                .is_some()
        );

        let pillar = service
            .advance_finalization_pillar_post_processing(1, 1)
            .expect("pillar acknowledgement completes finalization");
        assert_eq!(
            pillar.next_step.runtime_status,
            PbftFinalizationRuntimeStatus::Complete
        );
        assert!(pillar.next_step.complete);
        assert!(
            service
                .manager_state()
                .finalization_runtime_session
                .is_none()
        );

        drop(service);
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn final_chain_advancement_derives_retained_blocks_per_year() {
        use crate::pbft_finalize::{
            PbftFinalizationCleanupIntent, PbftFinalizationRuntimeAction,
            PbftFinalizationRuntimeStatus,
        };

        let (path, storage) =
            temp_storage("rustaxa_consensus_pbft_service_finalization_final_chain");
        let service = PbftService::restore(storage, config(1)).unwrap();
        install_finalization_executor(
            &service,
            2,
            PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: false,
                reset_reward_votes: false,
                set_dag_block_order: false,
                update_sortition_params: false,
                update_finalized_transactions_status: false,
                update_pbft_chain: false,
                clear_anchor_dag_cache: false,
                finalize_final_chain: true,
                maybe_update_dynamic_lambda: false,
                advance_period: false,
                process_pillar_block: false,
            },
            vec![PbftFinalizationRuntimeAction::FinalizeFinalChain],
        );

        let boundary = service
            .advance_finalization_final_chain_dispatch(0, 2)
            .expect("retained blocks-per-year validates FinalChain dispatch");
        assert_eq!(
            boundary.next_step.runtime_status,
            PbftFinalizationRuntimeStatus::Complete
        );
        assert!(boundary.next_step.complete);
        assert!(
            service
                .manager_state()
                .finalization_runtime_session
                .is_none()
        );

        drop(service);
        let _ = fs::remove_dir_all(path);
    }
}
