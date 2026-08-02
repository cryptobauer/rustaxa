//! PBFT vote admission runtime for Rust-owned vote state and payloads.
//!
//! This module is the stateful companion to the side-effect-free PBFT vote
//! admission and progress planners. It owns the verified-vote index plus the
//! canonical/weighted payload sidecar needed by storage and slashing effects.
//! Bridge composition may enrich validation from FinalChain before entering
//! this runtime; admission compatibility callers can still supply explicit
//! validation facts. The runtime owns deterministic mutation ordering, replay
//! and validator-key caches, and payload bytes derived from admitted votes.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result, anyhow, ensure};
use ethereum_types::H256;
use rlp::Rlp;
use rustaxa_storage::{Column, ExtraRewardVotesGuard, Storage, StoredFinalizedRewardVoteCursor};
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::pbft::PbftBlockLink;

use crate::pbft_finalize::apply_pbft_reward_votes_reset_storage_locked;
use crate::pbft_finalize::{
    PbftFinalizationStorageWriteStage, PbftFinalizedPeriodApplyResult,
    PbftRewardVotesResetStorageRequest,
};
use crate::pbft_reward_votes::{
    PbftRewardVoteRoundCandidate, PbftRewardVoteSelectionFact, PbftRewardVoteSelectionPlan,
    PbftRewardVotesStatus, plan_pbft_reward_votes,
};
use crate::pbft_thresholds::{
    PbftTwoTPlusOneThresholdFact, PbftTwoTPlusOneThresholdPlan, PbftTwoTPlusOneThresholdRuntime,
};
use crate::pbft_vote_admission::{
    PbftVoteAdmissionExecution, PbftVoteAdmissionPrecheck, PbftVoteAdmissionSession,
};
use crate::pbft_vote_event::PbftVoteEventFactFlags;
use crate::pbft_vote_payload::{
    PbftVotePayloadRecord, build_optimized_pbft_vote_bundle, build_slashing_pbft_vote_payload,
    build_weighted_pbft_vote_bundle, build_weighted_pbft_vote_payload,
};
use crate::pbft_vote_progress::PbftVoteProgressContext;
use crate::pbft_vote_progress::PbftVoteProgressIntent;
use crate::pbft_vote_storage::{
    PbftTwoTPlusOneVoteBundle, PbftVotePersistenceResult, PbftVotePersistenceStatus,
    PbftVoteProgressPersistenceWrite, PbftVoteStorageRecord, clear_own_verified_votes,
    persist_pbft_vote_progress, save_own_verified_vote,
};
use crate::pbft_vote_validation::{
    PbftCanonicalVoteInspection, PbftCanonicalVoteInspectionStatus, PbftCanonicalVoteValidation,
    PbftVoteReplayCache, PbftVoteReplayCheckpoint, inspect_canonical_pbft_vote,
};
use crate::verified_votes::{
    AddVerifiedVoteOutcome, DetermineNewRoundOutcome as ConsensusDetermineNewRoundOutcome,
    RoundMarkerSnapshot, TwoTPlusOneSnapshotEntry, TwoTPlusOneVotedBlockType, VerifiedVote,
    VerifiedVotes, VerifiedVotesCleanupPlan, VerifiedVotesRoundCheckpoint, VotesWithWeight,
};

const PBFT_OPTIMIZED_BUNDLE_READY: VerifiedVoteOptimizedBundleStatus =
    VerifiedVoteOptimizedBundleStatus::Ready;
const PBFT_OPTIMIZED_BUNDLE_NOT_FOUND: VerifiedVoteOptimizedBundleStatus =
    VerifiedVoteOptimizedBundleStatus::NotFound;
const PBFT_OPTIMIZED_BUNDLE_EMPTY_REQUEST: VerifiedVoteOptimizedBundleStatus =
    VerifiedVoteOptimizedBundleStatus::EmptyRequest;
const PBFT_OPTIMIZED_BUNDLE_MAPPING_MISMATCH: VerifiedVoteOptimizedBundleStatus =
    VerifiedVoteOptimizedBundleStatus::MappingMismatch;
const PBFT_OPTIMIZED_BUNDLE_MISSING_PAYLOAD: VerifiedVoteOptimizedBundleStatus =
    VerifiedVoteOptimizedBundleStatus::MissingPayload;
const PBFT_OPTIMIZED_BUNDLE_PAYLOAD_DECODE_ERROR: VerifiedVoteOptimizedBundleStatus =
    VerifiedVoteOptimizedBundleStatus::PayloadDecodeError;
const PBFT_OPTIMIZED_BUNDLE_PAYLOAD_METADATA_MISMATCH: VerifiedVoteOptimizedBundleStatus =
    VerifiedVoteOptimizedBundleStatus::PayloadMetadataMismatch;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
/// Outcome of planning or building an optimized verified-vote bundle.
///
/// Every variant is deterministic protocol state, not an infrastructure
/// failure. The bridge projects variants to the stable legacy numeric and
/// error-code pair; lock, storage, and invariant failures remain `Err`.
pub enum VerifiedVoteOptimizedBundleStatus {
    /// A plan or encoded bundle is available.
    Ready = 0,
    /// No matching `2t+1` mapping exists.
    NotFound = 1,
    /// The caller requested no vote hashes.
    EmptyRequest = 2,
    /// Reserved bridge projection for an invalid raw compatibility kind.
    UnsupportedKind = 3,
    /// Requested block coordinates differ from the retained mapping.
    MappingMismatch = 4,
    /// A requested hash is absent from the retained plan.
    HashNotInPlan = 5,
    /// Requested hashes do not preserve retained plan order.
    OrderMismatch = 6,
    /// A retained plan hash has no weighted payload sidecar.
    MissingPayload = 7,
    /// A retained payload could not be decoded for optimized encoding.
    PayloadDecodeError = 8,
    /// A retained payload does not match the requested coordinates.
    PayloadMetadataMismatch = 9,
}

impl VerifiedVoteOptimizedBundleStatus {
    /// Returns the stable CXX numeric projection.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Returns the stable CXX diagnostic projection.
    pub const fn legacy_error_code(self) -> &'static str {
        match self {
            Self::Ready => "PBFT_OPTIMIZED_VOTE_BUNDLE_READY",
            Self::NotFound => "PBFT_OPTIMIZED_VOTE_BUNDLE_NOT_FOUND",
            Self::EmptyRequest => "PBFT_OPTIMIZED_VOTE_BUNDLE_EMPTY_REQUEST",
            Self::UnsupportedKind => "PBFT_OPTIMIZED_VOTE_BUNDLE_UNSUPPORTED_KIND",
            Self::MappingMismatch => "PBFT_OPTIMIZED_VOTE_BUNDLE_MAPPING_MISMATCH",
            Self::HashNotInPlan => "PBFT_OPTIMIZED_VOTE_BUNDLE_HASH_NOT_IN_PLAN",
            Self::OrderMismatch => "PBFT_OPTIMIZED_VOTE_BUNDLE_ORDER_MISMATCH",
            Self::MissingPayload => "PBFT_OPTIMIZED_VOTE_BUNDLE_MISSING_RETAINED_PAYLOAD",
            Self::PayloadDecodeError => "PBFT_OPTIMIZED_VOTE_BUNDLE_PAYLOAD_DECODE_ERROR",
            Self::PayloadMetadataMismatch => "PBFT_OPTIMIZED_VOTE_BUNDLE_PAYLOAD_METADATA_MISMATCH",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
/// Typed plan for one optimized verified-vote bundle family.
///
/// `plan` is absent only when the mapping is not found; its coordinates remain
/// the exact query coordinates. Infrastructure and retained-state invariant
/// failures are returned by the service instead of encoded here.
pub struct PbftOptimizedVoteBundlePlan {
    /// Stable status for this plan query.
    pub status: VerifiedVoteOptimizedBundleStatus,
    /// Bundle family requested in the plan.
    pub kind: TwoTPlusOneVotedBlockType,
    /// Requested PBFT period, retained even when no mapping exists.
    pub period: u64,
    /// Requested PBFT round, retained even when no mapping exists.
    pub round: u64,
    /// Full plan details when this family is available.
    pub plan: Option<PbftOptimizedVoteBundlePlanEntry>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
/// Available optimized-bundle coordinates and canonical vote-hash order.
///
/// Every hash belongs to the mapped bucket in retained insertion order; bundle
/// building accepts only ordered subsequences of this vector.
pub struct PbftOptimizedVoteBundlePlanEntry {
    /// Planned block hash.
    pub block_hash: H256,
    /// Planned period.
    pub period: u64,
    /// Planned round.
    pub round: u64,
    /// Planned step.
    pub step: u64,
    /// Ordered vote hashes for the plan.
    pub vote_hashes: Vec<H256>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
/// One verified vote paired with its required weighted payload sidecar.
///
/// Snapshot construction fails rather than emitting an entry when the retained
/// vote has no matching payload.
pub struct VerifiedVoteStateSnapshotEntry {
    /// Vote metadata retained in the verified-vote runtime.
    pub vote: VerifiedVote,
    /// Weighted vote payload retained alongside `vote` metadata.
    pub weighted_vote: PbftVotePayloadRecord,
}

#[derive(Debug, Clone, Eq, PartialEq)]
/// Coherent verified-vote metadata and payload snapshot from one lock epoch.
///
/// Entries and marker vectors preserve native deterministic ordering; missing
/// retained payloads make the complete snapshot fail.
pub struct VerifiedVotesStateSnapshot {
    /// Every retained vote with matching weighted payload.
    pub votes: Vec<VerifiedVoteStateSnapshotEntry>,
    /// Canonical round markers.
    pub round_markers: Vec<RoundMarkerSnapshot>,
    /// Canonical 2t+1 marker snapshot.
    pub two_t_plus_one: Vec<TwoTPlusOneSnapshotEntry>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
/// One block bucket from a step-level verified-vote payload snapshot.
///
/// `total_weight` is the retained aggregate for `votes`; every hash-resolved
/// payload must exist or the enclosing lookup fails.
pub struct VerifiedStepVotePayloadEntry {
    /// Voted block hash for this payload bucket.
    pub block_hash: H256,
    /// Total retained weight for this bucket.
    pub total_weight: u64,
    /// Retained weighted payloads in canonical order.
    pub votes: Vec<PbftVotePayloadRecord>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
/// Coherent next and next-null optimized-bundle plans for one query.
///
/// Both family plans are derived under one verified-vote lock epoch. Missing
/// mappings are represented by the optional entry inside each family plan.
pub struct PbftNextVotesBundleEgressPlan {
    /// Stable plan status.
    pub status: VerifiedVoteOptimizedBundleStatus,
    /// Period for both returned plans.
    pub period: u64,
    /// Round for both returned plans.
    pub round: u64,
    /// Next-vote 2t+1 plan.
    pub next_votes: PbftOptimizedVoteBundlePlan,
    /// Next-null 2t+1 plan.
    pub next_null_votes: PbftOptimizedVoteBundlePlan,
}

#[derive(Debug, Clone, Eq, PartialEq)]
/// Deterministic result of validating and encoding an optimized vote bundle.
///
/// Non-ready statuses carry no bundle bytes. Lock and invariant failures remain
/// service errors rather than status variants.
pub struct PbftOptimizedVoteBundleBuildResult {
    /// Stable build status.
    pub status: VerifiedVoteOptimizedBundleStatus,
    /// Verified vote hashes that were requested.
    pub vote_hashes: Vec<H256>,
    /// Optimized vote-bundle payload.
    pub votes_bundle_rlp: Vec<u8>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
/// Typed request to build an optimized bundle from retained vote hashes.
///
/// Hashes must be non-empty, match the mapped coordinates, and form an ordered
/// subsequence of the retained plan.
pub struct PbftOptimizedVoteBundleBuildRequest {
    /// Target bundle family.
    pub kind: TwoTPlusOneVotedBlockType,
    /// Target block hash.
    pub block_hash: H256,
    /// Target period.
    pub period: u64,
    /// Target round.
    pub round: u64,
    /// Target step.
    pub step: u64,
    /// Vote hashes in caller order.
    pub vote_hashes: Vec<H256>,
}

/// Native identity for a latest-round `2t+1` bundle persistence request.
///
/// The native runtime resolves the authoritative ordered weighted payloads
/// from its retained mapping. Callers cannot supply or materialize bundle
/// bytes across the service boundary.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVerifiedVoteProgressBundle {
    /// Domain bundle family.
    pub kind: TwoTPlusOneVotedBlockType,
    /// PBFT period represented by the bundle.
    pub period: u64,
    /// PBFT round represented by the bundle.
    pub round: u64,
    /// PBFT step that triggered persistence.
    pub step: u64,
    /// Block selected by the threshold family.
    pub block_hash: H256,
}

/// Native direct persistence request for PBFT vote-progress effects.
///
/// Either field may be absent. When present, both writes commit through the
/// existing Rust storage transaction while the verified-vote runtime lock
/// serializes the operation with admission persistence.
#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct PbftVerifiedVoteProgressPersistenceWrite {
    /// Optional accepted stale cert vote stored for reward selection, selected
    /// from the runtime's retained weighted payloads by canonical vote hash.
    pub extra_reward_vote_hash: Option<H256>,
    /// Optional latest-round `2t+1` bundle replacement.
    pub two_t_plus_one_bundle: Option<PbftVerifiedVoteProgressBundle>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
/// Mapped `2t+1` block coordinates with every retained weighted payload.
///
/// The service returns `None` for a missing mapping and fails if a present
/// mapping cannot resolve its complete payload set.
pub struct VerifiedVotesTwoTPlusOneVotePayloads {
    /// Target mapped voted block.
    pub block_hash: H256,
    /// Target mapped step.
    pub step: u64,
    /// Ordered retained vote payloads.
    pub votes: Vec<PbftVotePayloadRecord>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
/// Typed block and step returned by a `2t+1` mapping lookup.
///
/// Missing mappings are represented by `Option` at the service boundary.
pub struct VerifiedVotesTwoTPlusOneVotedBlock {
    /// Voted block.
    pub block_hash: H256,
    /// Source step.
    pub step: u64,
}

/// Canonical and weighted PBFT vote payloads retained for one admitted vote.
///
/// `slashing` is the unweighted signed vote payload used for double-vote proof
/// calldata. `weighted` is the storage payload used for own votes, extra reward
/// votes, and 2t+1 vote bundles. Both records use the same canonical vote hash.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteRuntimePayload {
    /// Unweighted signed PBFT vote payload for slashing evidence.
    pub slashing: PbftVotePayloadRecord,
    /// Weighted PBFT vote payload for storage.
    pub weighted: PbftVotePayloadRecord,
}

/// Rust-owned coordinates of the authoritative reward-vote certificate.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RewardVoteCursor {
    pub period: u64,
    pub round: u64,
    pub step: u64,
    pub block_hash: H256,
}

/// Result of committing a reward cursor after durable reset persistence.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RewardVoteCursorCommitResult {
    pub status: RewardVoteCursorCommitStatus,
    pub cursor: RewardVoteCursor,
    pub reset_generation: u64,
    pub error_code: &'static str,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RewardVoteCursorCommitStatus {
    /// A newer durable cursor was published to live state.
    Applied,
    /// The exact durable cursor was already current.
    AlreadyCurrent,
    /// Identity, generation, durability, or monotonicity validation failed.
    Rejected,
}

impl RewardVoteCursorCommitStatus {
    /// Returns the stable CXX status code.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Applied => 0,
            Self::AlreadyCurrent => 1,
            Self::Rejected => 2,
        }
    }
}

/// Reward-vote cursor snapshot used by Rust-native service code paths.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RewardVoteCursorSnapshot {
    /// Whether a persisted reward cursor exists.
    pub found: bool,
    /// Reward cursor period when present.
    pub period: u64,
    /// Reward cursor round when present.
    pub round: u64,
    /// Reward cursor step when present.
    pub step: u64,
    /// Reward cursor block hash when present.
    pub block_hash: H256,
}

/// Payload snapshot for the current reward-vote cursor and matching records.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RewardVotePayloadSnapshot {
    /// Reward cursor snapshot.
    pub cursor: RewardVoteCursorSnapshot,
    /// Canonical weighted payloads selected by the active cursor.
    pub records: Vec<PbftVotePayloadRecord>,
}

/// Service input for one native reward-vote reset transaction.
///
/// The identity must match the retained cert `2t+1` mapping exactly. `sync`
/// controls only RocksDB durability; it does not change validation, ordering,
/// or live cursor publication.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RewardVoteResetApplyRequest {
    /// Reward-vote period.
    pub period: u64,
    /// Reward-vote round.
    pub round: u64,
    /// Reward-vote step.
    pub step: u64,
    /// Reward-vote block hash.
    pub block_hash: H256,
    /// Storage sync flag forwarded to storage write-set apply.
    pub sync: bool,
}

/// Narrow native request for constructing a reward-vote reset stage.
///
/// `requested` preserves the finalization-plan gate while [`RewardVoteCursor`]
/// carries the exact cert identity that must match native verified-vote state.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RewardVoteResetPrepareRequest {
    /// Whether the accepted PBFT finalization plan requested a reward reset.
    pub requested: bool,
    /// Exact certified-vote identity whose weighted bundle must be retained.
    pub cursor: RewardVoteCursor,
}

/// Service input for publishing a cursor after a broader finalization batch.
///
/// The generation must be the active storage-authenticated reset generation.
/// The durable cursor and canonical bundle must match `cursor` byte-for-byte.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RewardVoteCursorCommitRequest {
    /// Reward-vote cursor to publish.
    pub cursor: RewardVoteCursor,
    /// Reward reset generation returned by reset storage apply.
    pub reset_generation: u64,
}

#[derive(Debug, Clone)]
struct RestoreVoteSource {
    vote_rlp: Vec<u8>,
}

fn inspect_restored_weighted_vote(vote_rlp: &[u8]) -> Result<PbftCanonicalVoteInspection> {
    let inspection = inspect_canonical_pbft_vote(vote_rlp)?;
    ensure!(
        inspection.status == PbftCanonicalVoteInspectionStatus::Valid && inspection.signature_valid,
        "stored verified vote is malformed or has an invalid signature"
    );
    ensure!(
        inspection.has_embedded_weight && inspection.embedded_weight > 0,
        "stored verified vote must contain a non-zero embedded weight"
    );
    Ok(inspection)
}

fn validate_restored_bundle_kind(
    kind: TwoTPlusOneVotedBlockType,
    vote: &PbftCanonicalVoteInspection,
) -> Result<()> {
    let valid = match kind {
        TwoTPlusOneVotedBlockType::SoftVotedBlock => {
            vote.vote_type == crate::verified_votes::PbftVoteType::Soft
        }
        TwoTPlusOneVotedBlockType::CertVotedBlock => {
            vote.vote_type == crate::verified_votes::PbftVoteType::Cert
        }
        TwoTPlusOneVotedBlockType::NextVotedBlock => {
            vote.vote_type == crate::verified_votes::PbftVoteType::Next
                && !vote.block_hash.is_zero()
        }
        TwoTPlusOneVotedBlockType::NextVotedNullBlock => {
            vote.vote_type == crate::verified_votes::PbftVoteType::Next && vote.block_hash.is_zero()
        }
    };
    ensure!(
        valid,
        "stored 2t+1 vote does not match bundle category {kind:?}"
    );
    Ok(())
}

fn validate_own_vote_storage_record(
    record: PbftVoteStorageRecord,
) -> Result<PbftVoteStorageRecord> {
    let inspection = inspect_canonical_pbft_vote(record.vote_rlp.as_slice())?;
    anyhow::ensure!(
        inspection.status == PbftCanonicalVoteInspectionStatus::Valid
            && inspection.signature_valid
            && inspection.has_embedded_weight
            && inspection.embedded_weight > 0,
        "VERIFIED_VOTES_OWN_VOTE_PAYLOAD_INVALID"
    );
    anyhow::ensure!(
        inspection.vote_hash == record.hash,
        "VERIFIED_VOTES_OWN_VOTE_HASH_MISMATCH"
    );
    Ok(record)
}

fn decode_finalized_reward_cursor_bundle(
    votes_bundle_rlp: &[u8],
) -> Result<StoredFinalizedRewardVoteCursor> {
    let rlp = Rlp::new(votes_bundle_rlp);
    ensure!(
        rlp.is_list() && rlp.item_count()? > 0,
        "legacy finalized reward cert bundle must be a non-empty RLP list"
    );
    let mut coordinates = None;
    let mut records = Vec::new();
    for vote in rlp.iter() {
        let inspection = inspect_restored_weighted_vote(vote.as_raw())?;
        validate_restored_bundle_kind(TwoTPlusOneVotedBlockType::CertVotedBlock, &inspection)?;
        let current = (
            inspection.period,
            inspection.round,
            inspection.step,
            inspection.block_hash,
        );
        ensure!(
            coordinates.is_none() || coordinates == Some(current),
            "legacy finalized reward cert bundle contains inconsistent vote coordinates"
        );
        coordinates = Some(current);
        records.push(PbftVotePayloadRecord {
            hash: inspection.vote_hash,
            vote_rlp: vote.as_raw().to_vec(),
        });
    }
    ensure!(
        build_weighted_pbft_vote_bundle(&records)? == votes_bundle_rlp,
        "legacy finalized reward cert bundle is not canonically encoded"
    );
    let (period, round, step, block_hash) = coordinates.expect("non-empty bundle checked");
    Ok(StoredFinalizedRewardVoteCursor {
        period,
        round,
        step,
        block_hash,
        votes_bundle_rlp: votes_bundle_rlp.to_vec(),
    })
}

fn merge_restore_source(
    sources: &mut BTreeMap<H256, RestoreVoteSource>,
    vote_hash: H256,
    vote_rlp: Vec<u8>,
) -> Result<()> {
    if let Some(existing) = sources.get_mut(&vote_hash) {
        ensure!(
            existing.vote_rlp == vote_rlp,
            "stored verified vote hash has inconsistent weighted payloads"
        );
    } else {
        sources.insert(vote_hash, RestoreVoteSource { vote_rlp });
    }
    Ok(())
}

/// Runtime-built 2t+1 vote bundle ready for storage persistence.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteRuntimeBundle {
    /// Threshold family whose vote bundle reached `2t+1`.
    pub kind: TwoTPlusOneVotedBlockType,
    /// PBFT period for the persisted bundle.
    pub period: u64,
    /// PBFT round for the persisted bundle.
    pub round: u64,
    /// PBFT step of the vote that triggered persistence.
    pub step: u64,
    /// Block hash selected by the threshold family.
    pub block_hash: H256,
    /// Number of weighted vote records included in the bundle.
    pub votes_count: usize,
    /// Raw legacy RLP list of weighted vote records.
    pub votes_bundle_rlp: Vec<u8>,
}

/// Runtime-built slashing payload pair for a duplicate-voter conflict.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteRuntimeSlashingPayloads {
    /// Incoming conflicting vote payload.
    pub incoming: PbftVotePayloadRecord,
    /// Existing conflicting vote payload from the runtime sidecar.
    pub conflicting: PbftVotePayloadRecord,
}

/// Rust-owned PBFT reward-vote selection with retained weighted payloads.
///
/// The selection mirrors legacy reward-vote lookup order while keeping the
/// candidate construction and payload resolution under the vote admission
/// runtime that owns verified-vote metadata and retained bytes.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftRewardVotePayloadSelection {
    /// True when the PBFT block reward-vote references are accepted.
    pub accepted: bool,
    /// Stable reward-vote selection status.
    pub status: PbftRewardVotesStatus,
    /// Reward period used for lookup.
    pub selected_period: u64,
    /// Round that satisfied the requested vote hashes.
    pub selected_round: u64,
    /// Reward block hash used for lookup.
    pub selected_block_hash: H256,
    /// Requested vote hashes in PBFT-block order when accepted.
    pub selected_vote_hashes: Vec<H256>,
    /// Retained weighted records in the same order as `selected_vote_hashes`.
    pub selected_records: Vec<PbftVotePayloadRecord>,
    /// First missing vote hash when selection failed or payload retention is incomplete.
    pub missing_vote_hash: Option<H256>,
}

/// Runtime replay-cache result for one validation or admission transition.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftVoteRuntimeReplayOutcome {
    /// Whether validation reached a state that should be replay-protected.
    pub should_mark: bool,
    /// Whether the runtime inserted a new replay-cache entry.
    pub inserted: bool,
    /// Whether the replay entry already existed before this transition.
    pub already_present: bool,
}

/// Complete result of one validation-backed vote admission transition.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteRuntimeAdmissionOutcome {
    /// Pre-mutation planner output, including validation and progress facts.
    pub precheck: PbftVoteAdmissionPrecheck,
    /// Replay-cache mutation result for this transition.
    pub replay: PbftVoteRuntimeReplayOutcome,
    /// Terminal execution plan after the runtime applied the verified-vote
    /// mutation, when a mutation was requested.
    pub execution: Option<PbftVoteAdmissionExecution>,
    /// Verified-vote insertion report, when one was applied.
    pub add_outcome: Option<AddVerifiedVoteOutcome>,
    /// Weighted storage record for this vote, when the vote was accepted and
    /// payload construction succeeded.
    pub storage_vote: Option<PbftVotePayloadRecord>,
    /// Weighted 2t+1 vote bundle, when threshold progress requested bundle
    /// persistence and all selected payload sidecars were present.
    pub two_t_plus_one_bundle: Option<PbftVoteRuntimeBundle>,
    /// Unweighted slashing evidence payloads, when a duplicate-voter conflict
    /// was reported and both payloads were available.
    pub slashing_payloads: Option<PbftVoteRuntimeSlashingPayloads>,
}

/// Durable publication status for one transactional admission.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftVoteAdmissionPersistenceStatus {
    /// The transition had no durable vote-progress effects.
    NotRequired,
    /// The complete persistence write committed before runtime publication.
    Applied,
    /// Persistence rejected or could not acquire/operate on storage.
    Rejected,
}

impl PbftVoteAdmissionPersistenceStatus {
    /// Stable bridge code for the publication result.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::NotRequired => 0,
            Self::Applied => 1,
            Self::Rejected => 2,
        }
    }
}

/// Transactional admission result after optional durable persistence.
///
/// `transition_published` is true only when the runtime mutation is visible
/// and every required storage write committed. Rejected persistence returns
/// the validation/admission diagnostic outcome for reporting, but runtime state
/// has already been restored and downstream effects must not be executed.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteAdmissionTransactionResult {
    pub outcome: PbftVoteRuntimeAdmissionOutcome,
    pub persistence_required: bool,
    pub persistence_status: PbftVoteAdmissionPersistenceStatus,
    pub persistence_applied_writes: u64,
    pub transition_published: bool,
    pub persistence_error_code: String,
}

struct PbftVoteAdmissionCheckpoint {
    replay: Option<PbftVoteReplayCheckpoint>,
    round: VerifiedVotesRoundCheckpoint,
    vote_hash: H256,
    previous_payload: Option<PbftVoteRuntimePayload>,
}

/// Exact, side-effect-free cleanup plan for verified votes and payloads.
///
/// The plan is captured while the runtime is locked, then applied only after
/// sibling proposed-block storage deletion commits. Application performs only
/// direct map removals and cannot fail.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteRuntimeCleanupPlan {
    verified_votes: VerifiedVotesCleanupPlan,
    stale_payload_hashes: Vec<H256>,
}

impl PbftVoteRuntimeCleanupPlan {
    /// First period retained after cleanup.
    pub const fn first_period_to_keep(&self) -> u64 {
        self.verified_votes.first_period_to_keep()
    }

    /// Number of verified-vote period maps removed.
    pub fn periods_removed(&self) -> u64 {
        self.verified_votes.periods_removed()
    }

    /// Number of verified votes removed.
    pub fn votes_removed(&self) -> u64 {
        self.verified_votes.votes_removed()
    }

    /// Number of retained payload sidecars removed.
    pub fn payloads_removed(&self) -> u64 {
        self.stale_payload_hashes.len() as u64
    }
}

/// Rust-owned PBFT vote admission state.
///
/// The runtime owns deterministic verified-vote metadata and the byte payloads
/// needed after admission. It does not own live C++ `PbftVote` objects, storage
/// handles, network peers, or transaction submission. Its address-keyed VRF
/// cache preserves validator-key lookup behavior for composed validation while
/// keeping FinalChain access outside the runtime lock.
#[derive(Debug, Clone)]
pub struct PbftVoteAdmissionRuntime {
    verified_votes: VerifiedVotes,
    replay_cache: PbftVoteReplayCache,
    threshold_runtime: PbftTwoTPlusOneThresholdRuntime,
    payloads: BTreeMap<H256, PbftVoteRuntimePayload>,
    reward_vote_cursor: Option<RewardVoteCursor>,
    reward_vote_cursor_reset_generation: u64,
    validation_vrf_keys: BTreeMap<[u8; 20], [u8; 32]>,
}

/// Native application owner for durable and in-memory PBFT verified-vote state.
///
/// The service restores one [`PbftVoteAdmissionRuntime`] before publication,
/// retains the exact [`Storage`] instance used for that restoration, and shares
/// both through cloneable Rust handles. Every clone observes one mutex-protected
/// runtime; callers must not hold its guard across external C++, network,
/// FinalChain/EVM, signing, or logging work. Storage failures prevent
/// construction, so a published service is always storage-capable.
#[derive(Clone)]
pub struct PbftVerifiedVotesService {
    storage: Arc<Storage>,
    runtime: Arc<Mutex<PbftVoteAdmissionRuntime>>,
}

/// Temporary task-neutral runtime guard returned by
/// [`PbftVerifiedVotesService::lock`].
///
/// This escape hatch supports adjacent PBFT workflows while their composition
/// moves native. Dropping the guard releases the single verified-vote lock; it
/// must not cross an external executor boundary.
pub type PbftVerifiedVotesServiceGuard<'a> = MutexGuard<'a, PbftVoteAdmissionRuntime>;

impl PbftVerifiedVotesService {
    /// Restores the service from durable storage in one constructor path.
    ///
    /// Inputs:
    /// - `storage`: shared durable handle for verified-vote columns.
    ///
    /// Outputs:
    /// - A cloneable service with restored runtime state and retained storage.
    ///
    /// Error behavior:
    /// - Storage and durability validation failures are returned as `anyhow::Error`.
    pub fn restore(storage: Arc<Storage>) -> Result<Self> {
        let runtime = PbftVoteAdmissionRuntime::restore_from_storage(storage.as_ref())?;
        Ok(Self {
            storage,
            runtime: Arc::new(Mutex::new(runtime)),
        })
    }

    /// Rebuilds a service from test-provided runtime state.
    ///
    /// This constructor is intentionally test-only and exists for focused,
    /// deterministic state injection without durability reads.
    #[cfg(test)]
    pub fn from_runtime(storage: Arc<Storage>, runtime: PbftVoteAdmissionRuntime) -> Self {
        Self {
            storage,
            runtime: Arc::new(Mutex::new(runtime)),
        }
    }

    /// Locks the single shared verified-vote runtime for a native operation.
    ///
    /// The returned guard exposes the restored admission runtime and serializes
    /// all state reads and mutations across service clones. Lock poisoning is
    /// returned as `PBFT_VERIFIED_VOTES_SERVICE_LOCK_POISONED`; callers must
    /// release the guard before invoking external executors.
    pub(crate) fn lock(&self) -> Result<PbftVerifiedVotesServiceGuard<'_>> {
        self.runtime
            .lock()
            .map_err(|_| anyhow!("PBFT_VERIFIED_VOTES_SERVICE_LOCK_POISONED"))
    }

    /// Borrows the durable storage retained by this service.
    ///
    /// The handle is the same instance used during restoration and remains
    /// valid for the service lifetime. It is temporarily exposed so adjacent
    /// atomic admission, cleanup, and finalization workflows can commit before
    /// publishing guarded memory changes.
    pub(crate) fn storage(&self) -> &Storage {
        self.storage.as_ref()
    }

    /// Loads locally generated weighted vote records from owned storage.
    ///
    /// Records are validated as payload-key-canonical weighted PBFT votes and
    /// only accepted when signatures are valid and embedded weight is positive.
    /// Rows are returned in storage-key order under the own-vote storage mutex;
    /// lock, enumeration, decoding, signature, and hash mismatches are errors.
    pub fn verified_votes_own_vote_records(&self) -> Result<Vec<PbftVoteStorageRecord>> {
        let _guard = self.storage.lock_own_verified_votes()?;
        self.storage
            .pbft()
            .own_verified_vote_records()?
            .into_iter()
            .map(|record| {
                validate_own_vote_storage_record(PbftVoteStorageRecord {
                    hash: record.vote_hash,
                    vote_rlp: record.vote_rlp,
                })
            })
            .collect()
    }

    /// Returns the in-memory verified-vote count from one runtime lock epoch.
    ///
    /// The result includes every retained vote hash. Runtime lock poison is
    /// returned as `PBFT_VERIFIED_VOTES_SERVICE_LOCK_POISONED`.
    pub fn verified_votes_size(&self) -> Result<u64> {
        Ok(self.lock()?.verified_votes().size())
    }

    /// Checks whether the supplied hash is already recorded in replay protection.
    ///
    /// The query is read under one runtime lock epoch and returns lock poison as
    /// an error without mutating replay state.
    pub fn verified_votes_replay_contains(&self, vote_hash: H256) -> Result<bool> {
        Ok(self.lock()?.replay_contains(vote_hash))
    }

    /// Inserts the supplied hash into replay protection.
    ///
    /// Returns `true` only for a new insertion and `false` for an existing hash;
    /// runtime lock poison is returned without mutation.
    pub fn verified_votes_replay_insert(&self, vote_hash: H256) -> Result<bool> {
        Ok(self.lock()?.replay_insert(vote_hash))
    }

    /// Calculates next-round advancement from stored next-vote `2t+1` mappings.
    ///
    /// `period` and `current_round` select the search window. `None` means no
    /// qualifying mapping; the complete read uses one runtime lock epoch.
    pub fn verified_votes_determine_new_round(
        &self,
        period: u64,
        current_round: u64,
    ) -> Result<Option<ConsensusDetermineNewRoundOutcome>> {
        Ok(self
            .lock()?
            .verified_votes()
            .determine_new_round(period, current_round))
    }

    /// Looks up one `2t+1` voted block marker for the supplied period/round.
    ///
    /// The typed `kind` selects the mapping family. `None` is an ordinary
    /// missing mapping; runtime lock poison is the only operational error.
    pub fn verified_votes_get_two_t_plus_one_voted_block(
        &self,
        period: u64,
        round: u64,
        kind: TwoTPlusOneVotedBlockType,
    ) -> Result<Option<VerifiedVotesTwoTPlusOneVotedBlock>> {
        Ok(self
            .lock()?
            .verified_votes()
            .get_two_t_plus_one_voted_block(period, round, kind)
            .map(|value| VerifiedVotesTwoTPlusOneVotedBlock {
                block_hash: value.hash,
                step: value.step,
            }))
    }

    /// Returns retained weighted payloads for one mapped `2t+1` voted block.
    ///
    /// `None` means the typed mapping is absent. A present mapping must resolve
    /// every ordered payload under the same lock epoch or the lookup fails.
    pub fn verified_votes_get_two_t_plus_one_voted_block_payloads(
        &self,
        period: u64,
        round: u64,
        kind: TwoTPlusOneVotedBlockType,
    ) -> Result<Option<VerifiedVotesTwoTPlusOneVotePayloads>> {
        let runtime = self.lock()?;
        let Some(voted_block) = runtime
            .verified_votes()
            .get_two_t_plus_one_voted_block(period, round, kind)
        else {
            return Ok(None);
        };
        let votes = runtime
            .two_t_plus_one_weighted_payloads(period, round, kind)?
            .ok_or(anyhow!("PBFT_VERIFIED_VOTES_TWO_T_PLUS_ONE_MISSING"))?
            .into_iter()
            .collect();
        Ok(Some(VerifiedVotesTwoTPlusOneVotePayloads {
            block_hash: voted_block.hash,
            step: voted_block.step,
            votes,
        }))
    }

    /// Persists one latest-round own verified vote through this service.
    ///
    /// `canonical_vote_rlp` is the signed, unweighted legacy vote encoding and
    /// `weight` is the authoritative nonzero sortition weight. Rust constructs
    /// the canonical weighted record, validates its signature, embedded weight,
    /// and derived hash, then commits it through the own-vote storage
    /// transaction. Malformed bytes, invalid signatures, and zero weight are
    /// `Err`; write failures use the stable rejected persistence result.
    pub fn verified_votes_save_own_verified_vote(
        &self,
        canonical_vote_rlp: &[u8],
        weight: u64,
    ) -> Result<PbftVotePersistenceResult> {
        let record = build_weighted_pbft_vote_payload(canonical_vote_rlp, weight)?;
        save_own_verified_vote(
            self.storage(),
            validate_own_vote_storage_record(PbftVoteStorageRecord {
                hash: record.hash,
                vote_rlp: record.vote_rlp,
            })?,
        )
    }

    /// Clears all latest-round own verified votes through this service.
    ///
    /// Enumeration and batch commit share the own-vote storage mutex. Lock or
    /// enumeration errors propagate; write failures use a rejected result.
    pub fn verified_votes_clear_own_verified_votes(&self) -> Result<PbftVotePersistenceResult> {
        clear_own_verified_votes(self.storage())
    }

    /// Persists generated vote-progress writes through this service.
    ///
    /// The optional extra-reward hash and 2t+1 coordinates identify payloads
    /// already retained by this runtime. Under one runtime guard, Rust resolves
    /// the reward record, validates the exact mapped block and step, builds the
    /// ordered canonical bundle, and commits both effects atomically. Missing
    /// payloads, mappings, or identity mismatches are `Err`; storage write
    /// failures remain typed rejected results. An empty request is a successful
    /// zero-write transaction.
    pub fn verified_votes_persist_pbft_vote_progress(
        &self,
        write: PbftVerifiedVoteProgressPersistenceWrite,
    ) -> Result<PbftVotePersistenceResult> {
        let runtime = self.lock()?;
        let extra_reward_vote = write
            .extra_reward_vote_hash
            .map(|vote_hash| {
                runtime
                    .weighted_payload(vote_hash)
                    .cloned()
                    .map(|record| PbftVoteStorageRecord {
                        hash: record.hash,
                        vote_rlp: record.vote_rlp,
                    })
                    .ok_or_else(|| anyhow!("PBFT_VOTE_PROGRESS_EXTRA_REWARD_PAYLOAD_MISSING"))
            })
            .transpose()?;
        let two_t_plus_one_bundle = write
            .two_t_plus_one_bundle
            .map(|bundle| {
                let mapping = runtime
                    .verified_votes()
                    .get_two_t_plus_one_voted_block(bundle.period, bundle.round, bundle.kind)
                    .ok_or_else(|| anyhow!("PBFT_VOTE_PROGRESS_TWO_T_PLUS_ONE_MAPPING_MISSING"))?;
                ensure!(
                    mapping.hash == bundle.block_hash && mapping.step == bundle.step,
                    "PBFT_VOTE_PROGRESS_TWO_T_PLUS_ONE_IDENTITY_MISMATCH"
                );
                let records = runtime
                    .two_t_plus_one_weighted_payloads(bundle.period, bundle.round, bundle.kind)?
                    .ok_or_else(|| anyhow!("PBFT_VOTE_PROGRESS_TWO_T_PLUS_ONE_MAPPING_MISSING"))?;
                Ok(PbftTwoTPlusOneVoteBundle {
                    kind: bundle.kind.into(),
                    period: bundle.period,
                    round: bundle.round,
                    step: bundle.step,
                    block_hash: bundle.block_hash,
                    votes_bundle_rlp: build_weighted_pbft_vote_bundle(&records)?,
                })
            })
            .transpose()?;
        persist_pbft_vote_progress(
            self.storage(),
            PbftVoteProgressPersistenceWrite {
                extra_reward_vote,
                two_t_plus_one_bundle,
            },
        )
    }

    /// Plans and applies one bounded verified-vote cleanup cycle.
    ///
    /// Votes strictly older than `pbft_period` are removed with their payloads
    /// under one runtime guard. Lock poison aborts without partial cleanup.
    pub fn verified_votes_cleanup_votes_by_period(&self, pbft_period: u64) -> Result<()> {
        self.lock()?.cleanup_votes_by_period(pbft_period);
        Ok(())
    }

    /// Builds next-vote bundle egress plans with one lock epoch.
    ///
    /// `period` and `round` are preserved in both typed family plans, including
    /// missing mappings. Both plans observe one coherent runtime snapshot;
    /// retained-state or lock failures propagate.
    pub fn verified_votes_plan_next_votes_bundle_egress(
        &self,
        period: u64,
        round: u64,
    ) -> Result<PbftNextVotesBundleEgressPlan> {
        let runtime = self.lock()?;
        Ok(PbftNextVotesBundleEgressPlan {
            status: PBFT_OPTIMIZED_BUNDLE_READY,
            period,
            round,
            next_votes: self.verified_votes_optimized_bundle_plan_runtime(
                &runtime,
                period,
                round,
                TwoTPlusOneVotedBlockType::NextVotedBlock,
            )?,
            next_null_votes: self.verified_votes_optimized_bundle_plan_runtime(
                &runtime,
                period,
                round,
                TwoTPlusOneVotedBlockType::NextVotedNullBlock,
            )?,
        })
    }

    /// Builds an optimized votes bundle egress payload from runtime state.
    ///
    /// The request must match the typed mapping and contain an ordered non-empty
    /// hash subsequence. Protocol mismatches return typed statuses; runtime lock
    /// and retained-state invariant failures remain errors.
    pub fn verified_votes_build_optimized_votes_bundle_egress(
        &self,
        request: PbftOptimizedVoteBundleBuildRequest,
    ) -> Result<PbftOptimizedVoteBundleBuildResult> {
        let requested_hashes = request.vote_hashes;
        if requested_hashes.is_empty() {
            return Ok(PbftOptimizedVoteBundleBuildResult {
                status: PBFT_OPTIMIZED_BUNDLE_EMPTY_REQUEST,
                vote_hashes: Vec::new(),
                votes_bundle_rlp: Vec::new(),
            });
        }

        let runtime = self.lock()?;
        let Some(voted) = runtime.verified_votes().get_two_t_plus_one_voted_block(
            request.period,
            request.round,
            request.kind,
        ) else {
            return Ok(PbftOptimizedVoteBundleBuildResult {
                status: PBFT_OPTIMIZED_BUNDLE_NOT_FOUND,
                vote_hashes: requested_hashes,
                votes_bundle_rlp: Vec::new(),
            });
        };

        if voted.hash != request.block_hash || voted.step != request.step {
            return Ok(PbftOptimizedVoteBundleBuildResult {
                status: PBFT_OPTIMIZED_BUNDLE_MAPPING_MISMATCH,
                vote_hashes: requested_hashes,
                votes_bundle_rlp: Vec::new(),
            });
        }

        let planned_hashes = runtime
            .verified_votes()
            .get_two_t_plus_one_voted_block_vote_hashes(
                request.period,
                request.round,
                request.kind,
            );
        if let Err(status) = ensure_ordered_subsequence(&planned_hashes, &requested_hashes) {
            return Ok(PbftOptimizedVoteBundleBuildResult {
                status,
                vote_hashes: requested_hashes,
                votes_bundle_rlp: Vec::new(),
            });
        }

        let mut records = Vec::with_capacity(requested_hashes.len());
        for vote_hash in &requested_hashes {
            let Some(record) = runtime.weighted_payload(*vote_hash).cloned() else {
                return Ok(PbftOptimizedVoteBundleBuildResult {
                    status: PBFT_OPTIMIZED_BUNDLE_MISSING_PAYLOAD,
                    vote_hashes: requested_hashes,
                    votes_bundle_rlp: Vec::new(),
                });
            };
            records.push(record);
        }

        match build_optimized_pbft_vote_bundle(
            &records,
            voted.hash,
            request.period,
            request.round,
            voted.step,
        ) {
            Ok(bundle) => Ok(PbftOptimizedVoteBundleBuildResult {
                status: PBFT_OPTIMIZED_BUNDLE_READY,
                vote_hashes: bundle.vote_hashes,
                votes_bundle_rlp: bundle.bundle_rlp,
            }),
            Err(err) if err.to_string().contains("mismatches requested metadata") => {
                Ok(PbftOptimizedVoteBundleBuildResult {
                    status: PBFT_OPTIMIZED_BUNDLE_PAYLOAD_METADATA_MISMATCH,
                    vote_hashes: requested_hashes,
                    votes_bundle_rlp: Vec::new(),
                })
            }
            Err(_) => Ok(PbftOptimizedVoteBundleBuildResult {
                status: PBFT_OPTIMIZED_BUNDLE_PAYLOAD_DECODE_ERROR,
                vote_hashes: requested_hashes,
                votes_bundle_rlp: Vec::new(),
            }),
        }
    }

    /// Returns deterministic verified-vote payload snapshots.
    ///
    /// Votes, round markers, and `2t+1` markers come from one runtime lock epoch.
    /// Missing weighted sidecars and lock poison fail the complete snapshot.
    pub fn verified_votes_state_snapshot(&self) -> Result<VerifiedVotesStateSnapshot> {
        let runtime = self.lock()?;
        let votes = runtime
            .verified_votes()
            .snapshot_votes()
            .into_iter()
            .map(|vote| {
                let weighted_vote = runtime
                    .weighted_payload(vote.vote_hash)
                    .cloned()
                    .ok_or_else(|| anyhow!("PBFT_VERIFIED_VOTES_MISSING_RETAINED_PAYLOAD"))?;
                Ok(VerifiedVoteStateSnapshotEntry {
                    vote,
                    weighted_vote,
                })
            })
            .collect::<Result<Vec<_>, anyhow::Error>>()?;

        Ok(VerifiedVotesStateSnapshot {
            votes,
            round_markers: runtime.verified_votes().snapshot_round_markers(),
            two_t_plus_one: runtime.verified_votes().snapshot_two_t_plus_one(),
        })
    }

    /// Returns one step's buckets of retained payloads in deterministic order.
    ///
    /// `None` means the `(period, round, step)` bucket is absent. A present bucket
    /// must resolve every weighted payload under the same lock or the lookup fails.
    pub fn verified_votes_step_payloads(
        &self,
        period: u64,
        round: u64,
        step: u64,
    ) -> Result<Option<Vec<VerifiedStepVotePayloadEntry>>> {
        let runtime = self.lock()?;
        let Some(entries) = runtime.verified_votes().get_step_votes(period, round, step) else {
            return Ok(None);
        };

        let entries = entries
            .into_iter()
            .map(|entry| {
                let votes = entry
                    .vote_hashes
                    .into_iter()
                    .map(|vote_hash| {
                        runtime.weighted_payload(vote_hash).cloned().ok_or_else(|| {
                            anyhow!("PBFT_VERIFIED_VOTES_STEP_MISSING_RETAINED_PAYLOAD")
                        })
                    })
                    .collect::<Result<Vec<_>, anyhow::Error>>()?;
                Ok(VerifiedStepVotePayloadEntry {
                    block_hash: entry.block_hash,
                    total_weight: entry.total_weight,
                    votes,
                })
            })
            .collect::<Result<Vec<_>, anyhow::Error>>()?;
        Ok(Some(entries))
    }

    fn verified_votes_optimized_bundle_plan_runtime(
        &self,
        runtime: &PbftVerifiedVotesServiceGuard<'_>,
        period: u64,
        round: u64,
        kind: TwoTPlusOneVotedBlockType,
    ) -> Result<PbftOptimizedVoteBundlePlan> {
        let _ = &self;
        let Some(voted) = runtime
            .verified_votes()
            .get_two_t_plus_one_voted_block(period, round, kind)
        else {
            return Ok(PbftOptimizedVoteBundlePlan {
                status: PBFT_OPTIMIZED_BUNDLE_NOT_FOUND,
                kind,
                period,
                round,
                plan: None,
            });
        };

        Ok(PbftOptimizedVoteBundlePlan {
            status: PBFT_OPTIMIZED_BUNDLE_READY,
            kind,
            period,
            round,
            plan: Some(PbftOptimizedVoteBundlePlanEntry {
                block_hash: voted.hash,
                period,
                round,
                step: voted.step,
                vote_hashes: runtime
                    .verified_votes()
                    .get_two_t_plus_one_voted_block_vote_hashes(period, round, kind),
            }),
        })
    }

    /// Returns the current reward-vote cursor as an owned domain snapshot.
    ///
    /// No inputs or storage reads are required. Before the first reset, the
    /// result has `found == false` and zero identity fields. Lock poison is
    /// returned without exposing partial state.
    pub fn reward_vote_cursor_snapshot(&self) -> Result<RewardVoteCursorSnapshot> {
        let runtime = self.lock()?;
        Ok(runtime
            .reward_vote_cursor()
            .map(|cursor| RewardVoteCursorSnapshot {
                found: true,
                period: cursor.period,
                round: cursor.round,
                step: cursor.step,
                block_hash: cursor.block_hash,
            })
            .unwrap_or_else(|| RewardVoteCursorSnapshot {
                found: false,
                period: 0,
                round: 0,
                step: 0,
                block_hash: H256::zero(),
            }))
    }

    /// Returns the finalized reward-vote period, or zero before the first reset.
    ///
    /// The value is read under the service mutex so clones observe the same
    /// cursor epoch. Lock poison is returned as an operational error.
    pub fn reward_vote_period(&self) -> Result<u64> {
        let runtime = self.lock()?;
        Ok(runtime.reward_vote_period())
    }

    /// Returns the cursor and its exact retained weighted payloads coherently.
    ///
    /// Cursor metadata and payloads are copied during one runtime lock epoch.
    /// An absent cursor yields an empty record list; a present cursor with a
    /// missing mapping or payload is an invariant error. Lock poison is
    /// reported without returning a partial snapshot.
    pub fn current_reward_vote_snapshot(&self) -> Result<RewardVotePayloadSnapshot> {
        let runtime = self.lock()?;
        let cursor = runtime
            .reward_vote_cursor()
            .map(|cursor| RewardVoteCursorSnapshot {
                found: true,
                period: cursor.period,
                round: cursor.round,
                step: cursor.step,
                block_hash: cursor.block_hash,
            })
            .unwrap_or_else(|| RewardVoteCursorSnapshot {
                found: false,
                period: 0,
                round: 0,
                step: 0,
                block_hash: H256::zero(),
            });
        let records = runtime
            .current_reward_vote_payloads()?
            .into_iter()
            .collect();
        Ok(RewardVotePayloadSnapshot { cursor, records })
    }

    /// Selects reward-vote payloads in caller-requested hash order.
    ///
    /// The native runtime owns preferred/reverse-round selection and payload
    /// resolution. Rejected protocol outcomes are returned as typed selection
    /// statuses; missing retained bytes and lock poison are hard errors.
    pub fn select_reward_vote_payloads(
        &self,
        block_period: u64,
        requested_vote_hashes: Vec<H256>,
    ) -> Result<PbftRewardVotePayloadSelection> {
        let runtime = self.lock()?;
        runtime.select_reward_vote_payloads(block_period, requested_vote_hashes)
    }

    /// Builds a canonical reward-reset stage from an exact cert identity.
    ///
    /// The request gate must be set, the native cert mapping must match period,
    /// round, step, and block hash, and every weighted payload must be retained.
    /// The returned stage contains no caller-selected delete keys. Validation
    /// and lock failures leave runtime and storage unchanged.
    pub fn prepare_reward_votes_reset_stage(
        &self,
        request: RewardVoteResetPrepareRequest,
    ) -> Result<PbftFinalizationStorageWriteStage> {
        let runtime = self.lock()?;
        runtime.prepare_reward_votes_reset_stage(request)
    }

    /// Applies and publishes one complete native reward-vote reset.
    ///
    /// The service holds its runtime mutex while deriving the canonical bundle,
    /// applying the Rust-owned storage reset, and publishing the generation-
    /// bound live cursor. Storage owns authoritative stale-row enumeration and
    /// its atomic batch. Rejected storage statuses do not publish the cursor;
    /// lock, storage, or post-commit invariant failures are returned as errors
    /// and remain restart-safe because durable cursor restoration is native.
    pub fn apply_reward_votes_reset(
        &self,
        request: RewardVoteResetApplyRequest,
    ) -> Result<PbftFinalizedPeriodApplyResult> {
        let mut runtime = self.lock()?;
        let reward_votes_bundle_rlp = runtime.reward_votes_reset_bundle(
            request.period,
            request.round,
            request.step,
            request.block_hash,
        )?;
        let storage_request = PbftRewardVotesResetStorageRequest {
            period: request.period,
            round: request.round,
            step: request.step,
            block_hash: request.block_hash,
            reward_votes_bundle_rlp,
        };
        let cursor = RewardVoteCursor {
            period: request.period,
            round: request.round,
            step: request.step,
            block_hash: request.block_hash,
        };
        runtime.ensure_reward_vote_cursor_monotonic(cursor)?;
        let storage_guard = self.storage.lock_extra_reward_votes()?;
        let result = apply_pbft_reward_votes_reset_storage_locked(
            &self.storage,
            &storage_guard,
            storage_request,
            request.sync,
        )?;
        if result.status.is_success() {
            let cursor_result = runtime.commit_reward_vote_cursor_locked(
                &self.storage,
                &storage_guard,
                cursor,
                result.reward_votes_reset_generation,
            )?;
            ensure!(
                cursor_result.status != RewardVoteCursorCommitStatus::Rejected,
                cursor_result.error_code
            );
        }
        Ok(result)
    }

    /// Publishes a cursor already committed by a broader finalization batch.
    ///
    /// Rust revalidates the active reset generation, exact cert mapping,
    /// retained canonical bundle, durable cursor bytes, and monotonicity while
    /// holding the service mutex. Exact replay is `AlreadyCurrent`; every
    /// rejected result leaves live state unchanged.
    pub fn commit_reward_vote_cursor(
        &self,
        request: RewardVoteCursorCommitRequest,
    ) -> Result<RewardVoteCursorCommitResult> {
        let mut runtime = self.lock()?;
        runtime.commit_reward_vote_cursor(&self.storage, request.cursor, request.reset_generation)
    }
}

fn ensure_ordered_subsequence(
    planned_hashes: &[H256],
    requested_hashes: &[H256],
) -> Result<(), VerifiedVoteOptimizedBundleStatus> {
    let mut cursor = 0;
    for requested_hash in requested_hashes {
        let Some(relative_position) = planned_hashes[cursor..]
            .iter()
            .position(|planned_hash| planned_hash == requested_hash)
        else {
            if planned_hashes[..cursor].contains(requested_hash) {
                return Err(VerifiedVoteOptimizedBundleStatus::OrderMismatch);
            }
            return Err(VerifiedVoteOptimizedBundleStatus::HashNotInPlan);
        };
        cursor += relative_position + 1;
    }
    Ok(())
}

impl Default for PbftVoteAdmissionRuntime {
    fn default() -> Self {
        Self::new_with_replay_cache(1_000_000, 1_000)
    }
}

impl PbftVoteAdmissionRuntime {
    /// Builds the canonical reward-reset stage while the caller retains the
    /// verified-vote guard.
    ///
    /// The request must identify the exact retained cert mapping and all
    /// weighted payloads must remain available. This lock-held form lets a
    /// composed finalization task keep admission serialized until the returned
    /// bytes are durably committed.
    pub(crate) fn prepare_reward_votes_reset_stage(
        &self,
        request: RewardVoteResetPrepareRequest,
    ) -> Result<PbftFinalizationStorageWriteStage> {
        anyhow::ensure!(request.requested, "PBFT_REWARD_VOTES_RESET_NOT_REQUESTED");
        let reward_votes_bundle_rlp = self.reward_votes_reset_bundle(
            request.cursor.period,
            request.cursor.round,
            request.cursor.step,
            request.cursor.block_hash,
        )?;
        Ok(PbftFinalizationStorageWriteStage {
            stage: 4,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 0,
            has_sortition_params_change: false,
            sortition_params_change_period: 0,
            sortition_params_change_interval_efficiency: 0,
            sortition_params_change_threshold_upper: 0,
            has_reward_votes_reset: true,
            reward_votes_bundle_rlp,
            has_prepared_pillar_block: false,
            prepared_pillar_block_period: 0,
            prepared_pillar_block_rlp: Vec::new(),
            extra_reward_vote_hashes: Vec::new(),
        })
    }

    /// Creates an empty PBFT vote admission runtime.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty PBFT vote admission runtime with explicit replay-cache
    /// bounds.
    ///
    /// Inputs:
    /// - `replay_max_size`: maximum retained validated vote hashes.
    /// - `replay_delete_step`: number of oldest hashes removed after capacity
    ///   is crossed.
    ///
    /// Outputs:
    /// - Runtime owning verified-vote state, replay protection, threshold
    ///   cache, and retained vote payload sidecars.
    #[must_use]
    pub fn new_with_replay_cache(replay_max_size: usize, replay_delete_step: usize) -> Self {
        Self {
            verified_votes: VerifiedVotes::new(),
            replay_cache: PbftVoteReplayCache::new(replay_max_size, replay_delete_step),
            threshold_runtime: PbftTwoTPlusOneThresholdRuntime::new(),
            payloads: BTreeMap::new(),
            reward_vote_cursor: None,
            reward_vote_cursor_reset_generation: 0,
            validation_vrf_keys: BTreeMap::new(),
        }
    }

    /// Returns the address-keyed VRF key cached by composed vote validation.
    ///
    /// The cache intentionally mirrors the legacy `KeyManager` lifetime: a
    /// successfully resolved key is reused for that validator address until
    /// the PBFT vote runtime is recreated.
    #[must_use]
    pub fn validation_vrf_key(&self, voter: [u8; 20]) -> Option<[u8; 32]> {
        self.validation_vrf_keys.get(&voter).copied()
    }

    /// Publishes a successfully resolved validator VRF key for later validation.
    pub fn cache_validation_vrf_key(&mut self, voter: [u8; 20], key: [u8; 32]) {
        self.validation_vrf_keys.insert(voter, key);
    }

    /// Restores the authoritative vote runtime directly from native Rust storage.
    ///
    /// The restore reads own votes, extra reward votes, and each typed latest-round
    /// `2t+1` slot. Weighted payloads are decoded and signature-checked, overlaps
    /// are deduplicated by canonical vote hash, and contradictory bytes for one
    /// hash are rejected. Typed bundles must contain votes with identical
    /// coordinates and a vote type/block-nullness matching their storage key.
    ///
    /// Runtime metadata, payload retention, uniqueness indexes, `2t+1`
    /// mappings, and the reward cursor are reconstructed before return. A
    /// legacy database with no dedicated cursor bootstraps it once from the
    /// validated latest cert bundle under the reward-storage lock. Databases
    /// with neither record restore a `None` cursor; malformed or contradictory
    /// durable state fails without exposing a partial runtime.
    pub fn restore_from_storage(storage: &Storage) -> Result<Self> {
        let own_votes = storage
            .pbft()
            .own_verified_vote_records()
            .context("VERIFIED_VOTES_RESTORE_OWN_READ")?;
        let reward_votes = storage
            .pbft()
            .extra_reward_vote_records()
            .context("VERIFIED_VOTES_RESTORE_REWARD_READ")?;
        let mut bundles = storage
            .pbft()
            .two_t_plus_one_votes_bundles()
            .context("VERIFIED_VOTES_RESTORE_TWO_T_PLUS_ONE_READ")?;
        let mut reward_record = storage
            .pbft()
            .finalized_reward_vote_cursor()
            .context("VERIFIED_VOTES_RESTORE_REWARD_CURSOR_READ")?;
        if reward_record.is_none()
            && let Some(legacy_cert_bundle) = bundles
                .iter()
                .find(|bundle| bundle.kind == u8::from(TwoTPlusOneVotedBlockType::CertVotedBlock))
        {
            let candidate = decode_finalized_reward_cursor_bundle(
                legacy_cert_bundle.votes_bundle_rlp.as_slice(),
            )?;
            let head = crate::pbft_chain::load_persisted_pbft_chain_head_identity(storage)?
                .ok_or_else(|| {
                    anyhow!("legacy reward cursor bootstrap is ambiguous: PBFT head is missing")
                })?;
            ensure!(
                head.size != 0 && !head.last_pbft_block_hash.is_zero(),
                "legacy reward cursor bootstrap is ambiguous: PBFT head is empty"
            );
            ensure!(
                candidate.period == head.size
                    && candidate.block_hash == head.last_pbft_block_hash
                    && storage.period().by_pbft_hash(candidate.block_hash)? == Some(head.size),
                "legacy reward cursor bootstrap is ambiguous: cert bundle does not match finalized PBFT head"
            );
            let finalized_block =
                crate::pbft_chain::load_pbft_block_from_storage(storage, candidate.block_hash)?;
            ensure!(
                finalized_block.found && !finalized_block.block_rlp.is_empty(),
                "legacy reward cursor bootstrap is ambiguous: finalized PBFT block payload is missing"
            );
            let finalized_link =
                PbftBlockLink::try_from(SignedPbftBlockRlp::new(&finalized_block.block_rlp))
                    .context("VERIFIED_VOTES_RESTORE_LEGACY_REWARD_CURSOR_BLOCK")?;
            ensure!(
                finalized_link.period == candidate.period && finalized_link.period == head.size,
                "legacy reward cursor bootstrap is ambiguous: finalized PBFT block embedded period mismatch"
            );
            let _guard = storage.lock_extra_reward_votes()?;
            reward_record = storage.pbft().finalized_reward_vote_cursor()?;
            if reward_record.is_none() {
                ensure!(
                    storage
                        .get_raw(Column::LatestRoundTwoTPlusOneVotes, &[1])?
                        .as_deref()
                        == Some(candidate.votes_bundle_rlp.as_slice()),
                    "legacy finalized reward cert bundle changed during cursor bootstrap"
                );
                let mut batch = storage.create_write_batch();
                storage
                    .pbft()
                    .write_finalized_reward_vote_cursor_in_batch(&mut batch, candidate.clone())?;
                storage.commit_write_batch_with_sync(batch, false)?;
                reward_record = Some(candidate);
            }
        }
        if reward_record.is_none()
            && !bundles
                .iter()
                .any(|bundle| bundle.kind == u8::from(TwoTPlusOneVotedBlockType::CertVotedBlock))
            && let Some(head) = crate::pbft_chain::load_persisted_pbft_chain_head_identity(storage)?
        {
            ensure!(
                head.size == 0,
                "legacy reward cursor bootstrap is ambiguous: finalized PBFT head has no cert bundle"
            );
        }
        let reward_cursor = reward_record.as_ref().map(|cursor| RewardVoteCursor {
            period: cursor.period,
            round: cursor.round,
            step: cursor.step,
            block_hash: cursor.block_hash,
        });
        if let Some(record) = &reward_record
            && !bundles.iter().any(|bundle| {
                bundle.kind == u8::from(TwoTPlusOneVotedBlockType::CertVotedBlock)
                    && bundle.votes_bundle_rlp == record.votes_bundle_rlp
            })
        {
            bundles.push(rustaxa_storage::StoredTwoTPlusOneVotesBundle {
                kind: u8::from(TwoTPlusOneVotedBlockType::CertVotedBlock),
                votes_bundle_rlp: record.votes_bundle_rlp.clone(),
            });
        }

        let mut sources = BTreeMap::<H256, RestoreVoteSource>::new();
        let mut bundle_mappings = Vec::new();

        for bundle in bundles {
            let kind = TwoTPlusOneVotedBlockType::try_from(bundle.kind)?;
            let rlp = Rlp::new(&bundle.votes_bundle_rlp);
            ensure!(
                rlp.is_list() && rlp.item_count()? > 0,
                "stored 2t+1 vote bundle {kind:?} must be a non-empty RLP list"
            );
            let mut coordinates = None;
            for vote in rlp.iter() {
                let vote_rlp = vote.as_raw().to_vec();
                let inspection = inspect_restored_weighted_vote(&vote_rlp)?;
                validate_restored_bundle_kind(kind, &inspection)?;
                let current = (
                    inspection.period,
                    inspection.round,
                    inspection.step,
                    inspection.block_hash,
                );
                if let Some(expected) = coordinates {
                    ensure!(
                        expected == current,
                        "stored 2t+1 vote bundle {kind:?} contains inconsistent vote coordinates"
                    );
                } else {
                    coordinates = Some(current);
                }
                merge_restore_source(&mut sources, inspection.vote_hash, vote_rlp)?;
            }
            let (period, round, step, block_hash) = coordinates.expect("non-empty bundle checked");
            bundle_mappings.push((kind, period, round, step, block_hash));
        }

        for record in own_votes {
            let inspection = inspect_restored_weighted_vote(&record.vote_rlp)?;
            ensure!(
                inspection.vote_hash == record.vote_hash,
                "stored own verified vote hash does not match its storage key"
            );
            merge_restore_source(&mut sources, inspection.vote_hash, record.vote_rlp)?;
        }
        for record in reward_votes {
            let inspection = inspect_restored_weighted_vote(&record.vote_rlp)?;
            ensure!(
                inspection.vote_hash == record.vote_hash,
                "stored extra reward vote hash does not match its storage key"
            );
            merge_restore_source(&mut sources, inspection.vote_hash, record.vote_rlp)?;
        }

        let mut runtime = Self::new();
        for (vote_hash, source) in &sources {
            let inspection = inspect_restored_weighted_vote(&source.vote_rlp)?;
            ensure!(
                inspection.vote_hash == *vote_hash,
                "restored vote hash changed during decode"
            );
            let vote = VerifiedVote::new(
                inspection.vote_hash,
                inspection.block_hash,
                inspection.recovered_voter,
                inspection.period,
                inspection.round,
                inspection.step,
                inspection.vote_type,
                inspection.embedded_weight,
            )?;
            let outcome = runtime.verified_votes.add_verified_vote(vote, None)?;
            ensure!(
                outcome.inserted,
                "stored verified vote conflicts with restored uniqueness index"
            );
            let slashing = build_slashing_pbft_vote_payload(&source.vote_rlp)?;
            runtime.payloads.insert(
                *vote_hash,
                PbftVoteRuntimePayload {
                    slashing,
                    weighted: PbftVotePayloadRecord {
                        hash: *vote_hash,
                        vote_rlp: source.vote_rlp.clone(),
                    },
                },
            );
            runtime.replay_insert(*vote_hash);
        }

        for (kind, period, round, step, block_hash) in bundle_mappings {
            let outcome = runtime
                .verified_votes
                .insert_two_t_plus_one_voted_block(period, round, kind, block_hash, step);
            ensure!(
                outcome.round_found && outcome.inserted,
                "stored 2t+1 mapping could not be restored for {kind:?}"
            );
        }

        if let Some(cursor) = reward_cursor {
            ensure!(
                runtime
                    .verified_votes
                    .get_two_t_plus_one_voted_block(
                        cursor.period,
                        cursor.round,
                        TwoTPlusOneVotedBlockType::CertVotedBlock,
                    )
                    .map(|mapping| (mapping.step, mapping.hash))
                    == Some((cursor.step, cursor.block_hash)),
                "stored finalized reward cursor does not match its durable cert bundle"
            );
        }

        runtime.reward_vote_cursor = reward_cursor;
        runtime.reward_vote_cursor_reset_generation = 0;
        Ok(runtime)
    }

    /// Returns immutable access to the Rust verified-vote index.
    #[must_use]
    pub const fn verified_votes(&self) -> &VerifiedVotes {
        &self.verified_votes
    }

    /// Returns mutable access to the Rust verified-vote index.
    pub const fn verified_votes_mut(&mut self) -> &mut VerifiedVotes {
        &mut self.verified_votes
    }

    /// Returns the weighted storage payload for `vote_hash`, when retained.
    #[must_use]
    pub fn weighted_payload(&self, vote_hash: H256) -> Option<&PbftVotePayloadRecord> {
        self.payloads
            .get(&vote_hash)
            .map(|payload| &payload.weighted)
    }

    /// Retains a caller-supplied weighted payload for an already accepted metadata vote.
    ///
    /// The record is decoded and its canonical hash, voter, block, coordinates,
    /// vote type, signature, and embedded weight must exactly match `vote`.
    /// Existing byte-identical payloads are idempotent; conflicting bytes are
    /// rejected without replacing the current sidecar. Callers must invoke this
    /// only while serializing the metadata mutation that accepted the vote.
    pub fn retain_weighted_payload(
        &mut self,
        vote: &VerifiedVote,
        record: PbftVotePayloadRecord,
    ) -> Result<()> {
        let inspection = inspect_canonical_pbft_vote(&record.vote_rlp)?;
        ensure!(
            inspection.status == PbftCanonicalVoteInspectionStatus::Valid
                && inspection.signature_valid
                && inspection.has_embedded_weight,
            "PBFT_VERIFIED_VOTE_WEIGHTED_PAYLOAD_INVALID"
        );
        ensure!(
            record.hash == vote.vote_hash
                && inspection.vote_hash == vote.vote_hash
                && inspection.block_hash == vote.block_hash
                && inspection.recovered_voter == vote.voter
                && inspection.period == vote.period
                && inspection.round == vote.round
                && inspection.step == vote.step
                && inspection.vote_type == vote.vote_type
                && inspection.embedded_weight == vote.weight,
            "PBFT_VERIFIED_VOTE_WEIGHTED_PAYLOAD_METADATA_MISMATCH"
        );
        let slashing = build_slashing_pbft_vote_payload(&record.vote_rlp)?;
        if let Some(existing) = self.payloads.get(&vote.vote_hash) {
            ensure!(
                existing.weighted == record && existing.slashing == slashing,
                "PBFT_VERIFIED_VOTE_WEIGHTED_PAYLOAD_CONFLICT"
            );
            return Ok(());
        }
        self.payloads.insert(
            vote.vote_hash,
            PbftVoteRuntimePayload {
                slashing,
                weighted: record,
            },
        );
        Ok(())
    }

    /// Returns all retained weighted PBFT vote payloads in deterministic vote-hash order.
    ///
    /// The returned records are suitable for temporary legacy materialization
    /// and network egress. The runtime only exposes payloads whose
    /// verified-vote metadata still exists, because cleanup prunes payloads
    /// against the verified-vote snapshot.
    #[must_use]
    pub fn weighted_payloads(&self) -> Vec<PbftVotePayloadRecord> {
        self.payloads
            .values()
            .map(|payload| payload.weighted.clone())
            .collect()
    }

    /// Returns retained weighted payloads for one Rust-owned 2t+1 mapping.
    ///
    /// Inputs:
    /// - `period`, `round`, and `kind` select the 2t+1 voted-block mapping.
    ///
    /// Outputs:
    /// - `None` when the mapping is absent.
    /// - Ordered weighted records for the mapped voted block when present.
    ///
    /// Error behavior:
    /// - Missing retained payloads for mapped vote hashes are hard errors
    ///   because verified-vote metadata and payload retention must advance
    ///   together.
    pub fn two_t_plus_one_weighted_payloads(
        &self,
        period: u64,
        round: u64,
        kind: TwoTPlusOneVotedBlockType,
    ) -> Result<Option<Vec<PbftVotePayloadRecord>>> {
        if self
            .verified_votes
            .get_two_t_plus_one_voted_block(period, round, kind)
            .is_none()
        {
            return Ok(None);
        }

        let vote_hashes = self
            .verified_votes
            .get_two_t_plus_one_voted_block_vote_hashes(period, round, kind);
        let mut records = Vec::with_capacity(vote_hashes.len());
        for vote_hash in vote_hashes {
            records.push(self.weighted_payload(vote_hash).cloned().ok_or_else(|| {
                anyhow!("PBFT vote runtime missing weighted payload for 2t+1 vote {vote_hash:#x}")
            })?);
        }
        Ok(Some(records))
    }

    #[must_use]
    /// Returns the authoritative reward cursor, or `None` before the first
    /// durable cert reset is restored or committed.
    pub const fn reward_vote_cursor(&self) -> Option<RewardVoteCursor> {
        self.reward_vote_cursor
    }

    #[must_use]
    /// Returns the current reward period, preserving legacy period-zero
    /// behavior when no reward cursor exists.
    pub fn reward_vote_period(&self) -> u64 {
        self.reward_vote_cursor
            .map(|cursor| cursor.period)
            .unwrap_or(0)
    }

    /// Returns retained weighted payloads for the cursor's exact cert mapping.
    /// Missing mapping or payload state is an invariant error; no cursor
    /// returns an empty list.
    pub fn current_reward_vote_payloads(&self) -> Result<Vec<PbftVotePayloadRecord>> {
        let Some(cursor) = self.reward_vote_cursor else {
            return Ok(Vec::new());
        };
        self.two_t_plus_one_weighted_payloads(
            cursor.period,
            cursor.round,
            TwoTPlusOneVotedBlockType::CertVotedBlock,
        )?
        .ok_or_else(|| anyhow!("PBFT_REWARD_CURSOR_CERT_MAPPING_MISSING"))
    }

    fn reward_votes_reset_bundle(
        &self,
        period: u64,
        round: u64,
        step: u64,
        block_hash: H256,
    ) -> Result<Vec<u8>> {
        let kind = TwoTPlusOneVotedBlockType::CertVotedBlock;
        let mapping = self
            .verified_votes
            .get_two_t_plus_one_voted_block(period, round, kind)
            .ok_or_else(|| anyhow!("PBFT_REWARD_VOTES_RESET_CERT_MAPPING_MISSING"))?;
        ensure!(
            mapping.hash == block_hash && mapping.step == step,
            "PBFT_REWARD_VOTES_RESET_CERT_IDENTITY_MISMATCH"
        );
        let records = self
            .two_t_plus_one_weighted_payloads(period, round, kind)?
            .ok_or_else(|| anyhow!("PBFT_REWARD_VOTES_RESET_CERT_MAPPING_MISSING"))?;
        build_weighted_pbft_vote_bundle(&records)
    }

    /// Commits a post-storage reward cursor without performing durable writes.
    ///
    /// The supplied generation must be the active storage reset generation.
    /// Rust validates the exact cert mapping, retained payloads, byte-equal
    /// durable cert bundle, and strictly increasing period. Exact replay is
    /// idempotent. Every rejection leaves the cursor unchanged.
    pub fn commit_reward_vote_cursor(
        &mut self,
        storage: &Storage,
        cursor: RewardVoteCursor,
        reset_generation: u64,
    ) -> Result<RewardVoteCursorCommitResult> {
        let guard = storage.lock_extra_reward_votes()?;
        self.commit_reward_vote_cursor_locked(storage, &guard, cursor, reset_generation)
    }

    fn ensure_reward_vote_cursor_monotonic(&self, cursor: RewardVoteCursor) -> Result<()> {
        ensure!(
            !self
                .reward_vote_cursor
                .is_some_and(|current| current != cursor && cursor.period <= current.period),
            "PBFT_REWARD_CURSOR_NOT_MONOTONIC"
        );
        Ok(())
    }

    fn commit_reward_vote_cursor_locked(
        &mut self,
        storage: &Storage,
        _guard: &ExtraRewardVotesGuard<'_>,
        cursor: RewardVoteCursor,
        reset_generation: u64,
    ) -> Result<RewardVoteCursorCommitResult> {
        let rejected = |error_code| RewardVoteCursorCommitResult {
            status: RewardVoteCursorCommitStatus::Rejected,
            cursor,
            reset_generation,
            error_code,
        };
        if reset_generation == 0
            || reset_generation != storage.extra_reward_votes_reset_generation()
        {
            return Ok(rejected("PBFT_REWARD_CURSOR_RESET_GENERATION_MISMATCH"));
        }
        if self
            .reward_vote_cursor
            .is_some_and(|current| current != cursor)
            && reset_generation <= self.reward_vote_cursor_reset_generation
        {
            return Ok(rejected("PBFT_REWARD_CURSOR_RESET_GENERATION_CONSUMED"));
        }
        let mapping = self.verified_votes.get_two_t_plus_one_voted_block(
            cursor.period,
            cursor.round,
            TwoTPlusOneVotedBlockType::CertVotedBlock,
        );
        if mapping.map(|value| (value.step, value.hash)) != Some((cursor.step, cursor.block_hash)) {
            return Ok(rejected("PBFT_REWARD_CURSOR_CERT_MAPPING_MISMATCH"));
        }
        let records = match self.two_t_plus_one_weighted_payloads(
            cursor.period,
            cursor.round,
            TwoTPlusOneVotedBlockType::CertVotedBlock,
        ) {
            Ok(Some(records)) => records,
            Ok(None) | Err(_) => {
                return Ok(rejected("PBFT_REWARD_CURSOR_CERT_PAYLOADS_MISSING"));
            }
        };
        let expected_bundle = match build_weighted_pbft_vote_bundle(&records) {
            Ok(bundle) => bundle,
            Err(_) => return Ok(rejected("PBFT_REWARD_CURSOR_CERT_PAYLOADS_INVALID")),
        };
        let durable_cursor = storage.pbft().finalized_reward_vote_cursor()?;
        if !durable_cursor.is_some_and(|stored| {
            (stored.period, stored.round, stored.step, stored.block_hash)
                == (cursor.period, cursor.round, cursor.step, cursor.block_hash)
                && stored.votes_bundle_rlp == expected_bundle
        }) {
            return Ok(rejected("PBFT_REWARD_CURSOR_DURABLE_CERT_MISMATCH"));
        }
        match self.reward_vote_cursor {
            Some(current) if current == cursor => {
                if reset_generation < self.reward_vote_cursor_reset_generation {
                    return Ok(rejected("PBFT_REWARD_CURSOR_RESET_GENERATION_CONSUMED"));
                }
                self.reward_vote_cursor_reset_generation = reset_generation;
                Ok(RewardVoteCursorCommitResult {
                    status: RewardVoteCursorCommitStatus::AlreadyCurrent,
                    cursor,
                    reset_generation,
                    error_code: "",
                })
            }
            Some(current) if cursor.period <= current.period => {
                Ok(rejected("PBFT_REWARD_CURSOR_NOT_MONOTONIC"))
            }
            _ if reset_generation <= self.reward_vote_cursor_reset_generation => {
                Ok(rejected("PBFT_REWARD_CURSOR_RESET_GENERATION_CONSUMED"))
            }
            _ => {
                self.reward_vote_cursor = Some(cursor);
                self.reward_vote_cursor_reset_generation = reset_generation;
                Ok(RewardVoteCursorCommitResult {
                    status: RewardVoteCursorCommitStatus::Applied,
                    cursor,
                    reset_generation,
                    error_code: "",
                })
            }
        }
    }

    /// Selects PBFT reward votes and resolves retained weighted payloads.
    ///
    /// Inputs:
    /// - `block_period`: period of the PBFT block being validated.
    /// - `requested_vote_hashes`: hashes listed by the PBFT block, whose order
    ///   must be preserved for temporary C++ sidecar materialization.
    ///
    /// Outputs:
    /// - Selection is derived from the runtime-owned reward cursor.
    /// - Rejected selection statuses from the side-effect-free reward planner.
    /// - Accepted selection metadata plus retained weighted payload records in
    ///   requested-hash order.
    ///
    /// Error behavior:
    /// - If an accepted selected vote hash has no retained weighted payload,
    ///   this returns a hard error because production reward selection must
    ///   operate on the unified Rust runtime, not metadata-only compatibility
    ///   helper inserts.
    pub fn select_reward_vote_payloads(
        &self,
        block_period: u64,
        requested_vote_hashes: Vec<H256>,
    ) -> Result<PbftRewardVotePayloadSelection> {
        let Some(cursor) = self.reward_vote_cursor else {
            return self.resolve_reward_vote_payload_selection(plan_pbft_reward_votes(
                PbftRewardVoteSelectionFact {
                    block_period,
                    reward_period: 0,
                    preferred_reward_round: 0,
                    reward_block_hash: H256::zero(),
                    requested_vote_hashes,
                    has_preferred_round: false,
                    preferred_round: PbftRewardVoteRoundCandidate {
                        round: 0,
                        has_cert_step: false,
                        has_reward_block: false,
                        vote_hashes: Vec::new(),
                    },
                    has_reward_period: false,
                    period_rounds: Vec::new(),
                },
            ));
        };
        let reward_period = cursor.period;
        let preferred_reward_round = cursor.round;
        let reward_block_hash = cursor.block_hash;
        let preferred_round_lookup = self.verified_votes.reward_vote_round_candidate(
            reward_period,
            preferred_reward_round,
            reward_block_hash,
        );
        let has_preferred_round = preferred_round_lookup.is_some();
        let preferred_round =
            preferred_round_lookup.unwrap_or_else(|| PbftRewardVoteRoundCandidate {
                round: preferred_reward_round,
                has_cert_step: false,
                has_reward_block: false,
                vote_hashes: Vec::new(),
            });
        let period_rounds_lookup = self
            .verified_votes
            .reward_vote_period_candidates_rev(reward_period, reward_block_hash);
        let has_reward_period = period_rounds_lookup.is_some();
        let period_rounds = period_rounds_lookup.unwrap_or_default();
        let fact = PbftRewardVoteSelectionFact {
            block_period,
            reward_period,
            preferred_reward_round,
            reward_block_hash,
            requested_vote_hashes,
            has_preferred_round,
            preferred_round,
            has_reward_period,
            period_rounds,
        };
        let plan = plan_pbft_reward_votes(fact);
        self.resolve_reward_vote_payload_selection(plan)
    }

    fn resolve_reward_vote_payload_selection(
        &self,
        plan: PbftRewardVoteSelectionPlan,
    ) -> Result<PbftRewardVotePayloadSelection> {
        if !plan.accepted {
            return Ok(PbftRewardVotePayloadSelection {
                accepted: false,
                status: plan.status,
                selected_period: plan.selected_period,
                selected_round: plan.selected_round,
                selected_block_hash: plan.selected_block_hash,
                selected_vote_hashes: plan.selected_vote_hashes,
                selected_records: Vec::new(),
                missing_vote_hash: plan.missing_vote_hash,
            });
        }

        let mut records = Vec::with_capacity(plan.selected_vote_hashes.len());
        for vote_hash in &plan.selected_vote_hashes {
            records.push(self.weighted_payload(*vote_hash).cloned().ok_or_else(|| {
                anyhow!(
                    "PBFT reward-vote selection missing retained weighted payload for vote {vote_hash:#x}"
                )
            })?);
        }

        Ok(PbftRewardVotePayloadSelection {
            accepted: true,
            status: plan.status,
            selected_period: plan.selected_period,
            selected_round: plan.selected_round,
            selected_block_hash: plan.selected_block_hash,
            selected_vote_hashes: plan.selected_vote_hashes,
            selected_records: records,
            missing_vote_hash: None,
        })
    }

    /// Returns the slashing payload for `vote_hash`, when retained.
    #[must_use]
    pub fn slashing_payload(&self, vote_hash: H256) -> Option<&PbftVotePayloadRecord> {
        self.payloads
            .get(&vote_hash)
            .map(|payload| &payload.slashing)
    }

    /// Returns whether `vote_hash` is already retained in validation replay
    /// protection.
    #[must_use]
    pub fn replay_contains(&self, vote_hash: H256) -> bool {
        self.replay_cache.contains(vote_hash)
    }

    /// Inserts `vote_hash` into validation replay protection.
    ///
    /// The return value is true only when the hash was newly retained.
    pub fn replay_insert(&mut self, vote_hash: H256) -> bool {
        self.replay_cache.insert(vote_hash)
    }

    /// Applies replay-cache marking for one validation result.
    ///
    /// Inputs:
    /// - `validation`: canonical vote validation output carrying Rust's
    ///   replay-marker intent.
    ///
    /// Outputs:
    /// - Explicit replay decision and mutation facts so bridge callers do not
    ///   confuse "should mark" with "newly inserted."
    pub fn record_validation_replay(
        &mut self,
        validation: &PbftCanonicalVoteValidation,
    ) -> PbftVoteRuntimeReplayOutcome {
        let should_mark = validation.mark_validated_replay;
        if !should_mark {
            return PbftVoteRuntimeReplayOutcome {
                should_mark,
                inserted: false,
                already_present: false,
            };
        }
        let already_present = self.replay_contains(validation.vote_hash);
        let inserted = self.replay_insert(validation.vote_hash);
        PbftVoteRuntimeReplayOutcome {
            should_mark,
            inserted,
            already_present,
        }
    }

    /// Plans or computes the PBFT `2t+1` threshold from Rust-owned cache state.
    ///
    /// Inputs are explicit scalar facts supplied by the C++ boundary after
    /// FinalChain/PBFT-chain reads. The runtime owns cache lookup/update policy
    /// so threshold state is co-located with vote admission state.
    pub fn plan_two_t_plus_one_threshold(
        &mut self,
        fact: PbftTwoTPlusOneThresholdFact,
    ) -> PbftTwoTPlusOneThresholdPlan {
        self.threshold_runtime.plan_threshold(fact)
    }

    /// Plans removal of votes and payloads older than `pbft_period`.
    ///
    /// Planning does not mutate runtime state. Exact stale hashes let callers
    /// finish fallible durable work before publishing cleanup.
    pub fn plan_cleanup_votes_by_period(&self, pbft_period: u64) -> PbftVoteRuntimeCleanupPlan {
        let verified_votes = self
            .verified_votes
            .plan_cleanup_votes_by_period(pbft_period);
        let stale_payload_hashes = verified_votes
            .stale_vote_hashes()
            .iter()
            .copied()
            .filter(|vote_hash| self.payloads.contains_key(vote_hash))
            .collect();
        PbftVoteRuntimeCleanupPlan {
            verified_votes,
            stale_payload_hashes,
        }
    }

    /// Applies a captured vote/payload cleanup using direct removals only.
    pub fn apply_cleanup_votes_by_period(&mut self, plan: &PbftVoteRuntimeCleanupPlan) {
        self.verified_votes
            .apply_cleanup_votes_by_period(&plan.verified_votes);
        for vote_hash in &plan.stale_payload_hashes {
            self.payloads.remove(vote_hash);
        }
    }

    /// Removes votes and payloads for periods older than `pbft_period`.
    pub fn cleanup_votes_by_period(&mut self, pbft_period: u64) {
        let plan = self.plan_cleanup_votes_by_period(pbft_period);
        self.apply_cleanup_votes_by_period(&plan);
    }

    fn admission_checkpoint(
        &self,
        validation: &PbftCanonicalVoteValidation,
    ) -> PbftVoteAdmissionCheckpoint {
        PbftVoteAdmissionCheckpoint {
            replay: validation
                .mark_validated_replay
                .then(|| self.replay_cache.checkpoint_insert(validation.vote_hash)),
            round: self
                .verified_votes
                .checkpoint_round(validation.period, validation.round),
            vote_hash: validation.vote_hash,
            previous_payload: self.payloads.get(&validation.vote_hash).cloned(),
        }
    }

    fn restore_admission_checkpoint(&mut self, checkpoint: PbftVoteAdmissionCheckpoint) {
        if let Some(replay) = checkpoint.replay {
            self.replay_cache.restore_insert(replay);
        }
        self.verified_votes.restore_round(checkpoint.round);
        if let Some(payload) = checkpoint.previous_payload {
            self.payloads.insert(checkpoint.vote_hash, payload);
        } else {
            self.payloads.remove(&checkpoint.vote_hash);
        }
    }

    /// Admits and durably publishes one vote as a single runtime transaction.
    ///
    /// The runtime checkpoints only the replay insertion/eviction delta, the
    /// touched period/round, and the incoming hash's previous payload. It then
    /// executes normal admission, derives one existing vote-progress batch, and
    /// invokes `persist` only when durable effects exist. Applied and no-write
    /// transitions remain published. Rejected or operationally failed
    /// persistence restores the exact checkpoint and returns a typed rejected
    /// publication with no downstream effects authorized.
    ///
    /// The generic closure keeps failure injection stack-allocated for tests;
    /// no trait object or full-runtime clone is used.
    pub fn admit_validated_vote_transactional<F>(
        &mut self,
        canonical_vote_rlp: &[u8],
        validation: &PbftCanonicalVoteValidation,
        flags: PbftVoteEventFactFlags,
        context: PbftVoteProgressContext,
        persist: F,
    ) -> Result<PbftVoteAdmissionTransactionResult>
    where
        F: FnOnce(PbftVoteProgressPersistenceWrite) -> Result<PbftVotePersistenceResult>,
    {
        let checkpoint = self.admission_checkpoint(validation);
        let outcome =
            match self.admit_validated_vote(canonical_vote_rlp, validation, flags, context) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.restore_admission_checkpoint(checkpoint);
                    return Err(error);
                }
            };
        let write = match admission_persistence_write(&outcome) {
            Ok(write) => write,
            Err(_) => {
                self.restore_admission_checkpoint(checkpoint);
                return Ok(PbftVoteAdmissionTransactionResult {
                    outcome,
                    persistence_required: true,
                    persistence_status: PbftVoteAdmissionPersistenceStatus::Rejected,
                    persistence_applied_writes: 0,
                    transition_published: false,
                    persistence_error_code: "PBFT_VOTE_ADMISSION_INVALID_PERSISTENCE_PLAN"
                        .to_owned(),
                });
            }
        };
        let Some(write) = write else {
            return Ok(PbftVoteAdmissionTransactionResult {
                outcome,
                persistence_required: false,
                persistence_status: PbftVoteAdmissionPersistenceStatus::NotRequired,
                persistence_applied_writes: 0,
                transition_published: true,
                persistence_error_code: String::new(),
            });
        };
        match persist(write) {
            Ok(result) if result.status == PbftVotePersistenceStatus::Applied => {
                Ok(PbftVoteAdmissionTransactionResult {
                    outcome,
                    persistence_required: true,
                    persistence_status: PbftVoteAdmissionPersistenceStatus::Applied,
                    persistence_applied_writes: result.applied_writes,
                    transition_published: true,
                    persistence_error_code: result.error_code,
                })
            }
            Ok(result) => {
                self.restore_admission_checkpoint(checkpoint);
                Ok(PbftVoteAdmissionTransactionResult {
                    outcome,
                    persistence_required: true,
                    persistence_status: PbftVoteAdmissionPersistenceStatus::Rejected,
                    persistence_applied_writes: 0,
                    transition_published: false,
                    persistence_error_code: result.error_code,
                })
            }
            Err(_) => {
                self.restore_admission_checkpoint(checkpoint);
                Ok(PbftVoteAdmissionTransactionResult {
                    outcome,
                    persistence_required: true,
                    persistence_status: PbftVoteAdmissionPersistenceStatus::Rejected,
                    persistence_applied_writes: 0,
                    transition_published: false,
                    persistence_error_code: "PBFT_VOTE_PERSIST_STORAGE_OR_LOCK_FAILURE".to_owned(),
                })
            }
        }
    }

    /// Admits one validation-backed canonical PBFT vote into Rust-owned state.
    ///
    /// Inputs:
    /// - `canonical_vote_rlp`: legacy signed vote bytes supplied by the
    ///   boundary.
    /// - `validation`: accepted or rejected validation output for the same
    ///   canonical vote bytes.
    /// - `flags`: ingress/reward flags used by the vote-progress planner.
    /// - `context`: scalar PBFT vote-progress context, including optional
    ///   threshold and slashing enablement.
    ///
    /// Outputs:
    /// - Precheck/execution plans matching the existing session contract.
    /// - Rust-owned storage and slashing payloads for any requested side
    ///   effects.
    ///
    /// Edge behavior:
    /// - Rejected validation returns the terminal precheck without mutating
    ///   verified-vote state or payload sidecars.
    /// - Duplicate-voter conflicts do not insert the incoming vote, but still
    ///   return incoming and existing slashing payloads when available.
    /// - Missing retained payloads for a Rust-selected 2t+1 bundle or conflict
    ///   are hard errors because that indicates runtime state corruption.
    pub fn admit_validated_vote(
        &mut self,
        canonical_vote_rlp: &[u8],
        validation: &PbftCanonicalVoteValidation,
        mut flags: PbftVoteEventFactFlags,
        context: PbftVoteProgressContext,
    ) -> Result<PbftVoteRuntimeAdmissionOutcome> {
        flags.valid_stale_reward_vote = validation.period < context.current_period
            && validation.vote_type == crate::verified_votes::PbftVoteType::Cert
            && self.reward_vote_cursor.is_some_and(|cursor| {
                validation.period == cursor.period
                    && validation.block_hash == cursor.block_hash
                    && validation.round <= cursor.round.saturating_add(100)
            });
        let mut session = PbftVoteAdmissionSession::from_validation(validation, flags, context);
        let precheck = session.precheck();
        let replay = self.record_validation_replay(validation);
        if !precheck.should_insert() {
            return Ok(PbftVoteRuntimeAdmissionOutcome {
                precheck,
                replay,
                execution: None,
                add_outcome: None,
                storage_vote: None,
                two_t_plus_one_bundle: None,
                slashing_payloads: None,
            });
        }

        let fact = precheck.progress_fact.ok_or_else(|| {
            anyhow!("PBFT vote runtime precheck requested insert without progress fact")
        })?;
        let storage_vote = build_weighted_pbft_vote_payload(canonical_vote_rlp, fact.weight)?;
        let slashing_vote = build_slashing_pbft_vote_payload(canonical_vote_rlp)?;
        if storage_vote.hash != fact.identity.vote_hash
            || slashing_vote.hash != fact.identity.vote_hash
        {
            return Err(anyhow!(
                "PBFT vote runtime payload hash mismatches progress fact"
            ));
        }

        let vote = VerifiedVote::new(
            fact.identity.vote_hash,
            fact.identity.block_hash,
            fact.identity.voter,
            fact.identity.period,
            fact.identity.round,
            fact.identity.step,
            fact.vote_type,
            fact.weight,
        )?;
        let add_outcome = self
            .verified_votes
            .add_verified_vote(vote, context.two_t_plus_one_threshold)?;

        let execution = session.report_verified_vote_add(add_outcome);
        let mut retained_storage_vote = None;
        if add_outcome.inserted {
            self.payloads.insert(
                fact.identity.vote_hash,
                PbftVoteRuntimePayload {
                    slashing: slashing_vote.clone(),
                    weighted: storage_vote.clone(),
                },
            );
            retained_storage_vote = Some(storage_vote.clone());
        }

        let slashing_payloads = if let Some(conflicting_vote_hash) =
            add_outcome.conflicting_vote_hash
        {
            let conflicting = self
                    .slashing_payload(conflicting_vote_hash)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow!(
                            "PBFT vote runtime missing slashing payload for conflicting vote {conflicting_vote_hash:#x}"
                        )
                    })?;
            Some(PbftVoteRuntimeSlashingPayloads {
                incoming: slashing_vote,
                conflicting,
            })
        } else {
            None
        };

        let two_t_plus_one_bundle = if execution
            .pipeline_step
            .progress_plan
            .threshold_decision
            .is_some_and(|decision| {
                decision
                    .two_t_plus_one_insert_outcome
                    .is_some_and(|outcome| outcome.inserted)
            }) {
            let decision = execution
                .pipeline_step
                .progress_plan
                .threshold_decision
                .expect("checked above");
            let kind = decision
                .two_t_plus_one_kind
                .ok_or_else(|| anyhow!("PBFT vote runtime threshold inserted without kind"))?;
            let vote_hashes = self
                .verified_votes
                .get_two_t_plus_one_voted_block_vote_hashes(
                    fact.identity.period,
                    fact.identity.round,
                    kind,
                );
            let mut records = Vec::with_capacity(vote_hashes.len());
            for vote_hash in vote_hashes {
                records.push(self.weighted_payload(vote_hash).cloned().ok_or_else(|| {
                    anyhow!(
                        "PBFT vote runtime missing weighted payload for 2t+1 vote {vote_hash:#x}"
                    )
                })?);
            }
            Some(PbftVoteRuntimeBundle {
                kind,
                period: fact.identity.period,
                round: fact.identity.round,
                step: fact.identity.step,
                block_hash: fact.identity.block_hash,
                votes_count: records.len(),
                votes_bundle_rlp: build_weighted_pbft_vote_bundle(&records)?,
            })
        } else {
            None
        };

        Ok(PbftVoteRuntimeAdmissionOutcome {
            precheck,
            replay,
            execution: Some(execution),
            add_outcome: Some(add_outcome),
            storage_vote: retained_storage_vote,
            two_t_plus_one_bundle,
            slashing_payloads,
        })
    }

    /// Returns the vote bucket for a successfully inserted vote.
    ///
    /// This helper is used by the temporary C++ sidecar facade to reconstruct
    /// legacy `VotesWithWeight` objects from Rust metadata while the public C++
    /// API still returns live `PbftVote` pointers.
    #[must_use]
    pub fn inserted_votes_with_weight(&self, vote: &VerifiedVote) -> Option<VotesWithWeight> {
        self.verified_votes
            .get_step_votes(vote.period, vote.round, vote.step)?;
        self.verified_votes
            .votes_with_weight(vote.period, vote.round, vote.step, vote.block_hash)
    }
}

fn admission_persistence_write(
    outcome: &PbftVoteRuntimeAdmissionOutcome,
) -> Result<Option<PbftVoteProgressPersistenceWrite>> {
    let Some(execution) = outcome.execution.as_ref() else {
        return Ok(None);
    };
    let mut extra_reward_hash = None;
    let mut two_t_plus_one_identity = None;
    for intent in &execution.pipeline_step.progress_plan.intents {
        match intent {
            PbftVoteProgressIntent::PersistExtraRewardVote { vote_hash } => {
                ensure!(
                    extra_reward_hash.replace(*vote_hash).is_none(),
                    "duplicate extra-reward persistence intent"
                );
            }
            PbftVoteProgressIntent::PersistTwoTPlusOneVotes {
                kind,
                period,
                round,
                step,
                block_hash,
            } => {
                ensure!(
                    two_t_plus_one_identity
                        .replace((*kind, *period, *round, *step, *block_hash))
                        .is_none(),
                    "duplicate 2t+1 persistence intent"
                );
            }
            _ => {}
        }
    }
    if extra_reward_hash.is_none() && two_t_plus_one_identity.is_none() {
        return Ok(None);
    }
    let extra_reward_vote = extra_reward_hash
        .map(|expected_hash| {
            let vote = outcome
                .storage_vote
                .as_ref()
                .context("extra-reward persistence intent missing weighted vote")?;
            ensure!(
                vote.hash == expected_hash,
                "extra-reward persistence intent hash mismatch"
            );
            Ok(PbftVoteStorageRecord {
                hash: vote.hash,
                vote_rlp: vote.vote_rlp.clone(),
            })
        })
        .transpose()?;
    let two_t_plus_one_bundle = two_t_plus_one_identity
        .map(|(kind, period, round, step, block_hash)| {
            let bundle = outcome
                .two_t_plus_one_bundle
                .as_ref()
                .context("2t+1 persistence intent missing weighted bundle")?;
            ensure!(
                bundle.kind == kind
                    && bundle.period == period
                    && bundle.round == round
                    && bundle.step == step
                    && bundle.block_hash == block_hash,
                "2t+1 persistence intent identity mismatch"
            );
            Ok(PbftTwoTPlusOneVoteBundle {
                kind: bundle.kind.into(),
                period: bundle.period,
                round: bundle.round,
                step: bundle.step,
                block_hash: bundle.block_hash,
                votes_bundle_rlp: bundle.votes_bundle_rlp.clone(),
            })
        })
        .transpose()?;
    Ok(Some(PbftVoteProgressPersistenceWrite {
        extra_reward_vote,
        two_t_plus_one_bundle,
    }))
}

trait RuntimePrecheckExt {
    fn should_insert(&self) -> bool;
}

impl RuntimePrecheckExt for PbftVoteAdmissionPrecheck {
    fn should_insert(&self) -> bool {
        self.pipeline_step.as_ref().is_some_and(|step| {
            step.progress_plan.status
                == crate::pbft_vote_progress::PbftVoteProgressStatus::PendingVerifiedVoteInsert
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbft_finalize::PbftFinalizedPeriodApplyStatus;
    use crate::pbft_vote_event::PbftVoteEventFactFlags;
    use crate::pbft_vote_generation::{PbftVoteGenerationInput, generate_pbft_vote};
    use crate::pbft_vote_validation::{
        PbftVoteValidationExternalFacts, validate_canonical_pbft_vote,
    };
    use crate::verified_votes::{PbftVoteType, VerifiedVote};
    use ethereum_types::H160;
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_storage::{Config, Storage};
    use rustaxa_vdf::vrf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tiny_keccak::{Hasher, Keccak};

    const NODE_SECRET: [u8; 32] = [0x35; 32];
    const NODE_SECRET_TWO: [u8; 32] = [0x42; 32];
    const VRF_SECRET: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn voter_from_secret(secret: &[u8; 32]) -> [u8; 20] {
        let key = SigningKey::from_slice(secret).unwrap();
        let public_key = key.verifying_key().to_encoded_point(false);
        let mut output = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&public_key.as_bytes()[1..]);
        hasher.finalize(&mut output);
        output[12..].try_into().unwrap()
    }

    fn vote_rlp_from_secret(block_hash: [u8; 32], step: u64, node_secret: [u8; 32]) -> Vec<u8> {
        vote_rlp_for(block_hash, 12, 2, step, node_secret)
    }

    fn vote_rlp_for(
        block_hash: [u8; 32],
        period: u64,
        round: u64,
        step: u64,
        node_secret: [u8; 32],
    ) -> Vec<u8> {
        generate_pbft_vote(PbftVoteGenerationInput {
            block_hash: block_hash.into(),
            vote_type: PbftVoteType::Cert,
            period,
            round,
            step,
            node_secret,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&node_secret).into(),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap()
        .vote_rlp
    }

    fn vote_rlp(block_hash: [u8; 32], step: u64) -> Vec<u8> {
        vote_rlp_from_secret(block_hash, step, NODE_SECRET)
    }

    fn write_legacy_pbft_head(storage: &Storage, size: u64, block_hash: H256) {
        let json = format!(
            r#"{{"head_hash":"{:#x}","size":{},"non_empty_size":{},"last_pbft_block_hash":"{:#x}"}}"#,
            H256::zero(),
            size,
            size,
            block_hash
        );
        storage
            .pbft()
            .write_head(H256::zero(), json.as_bytes())
            .unwrap();
    }

    fn write_finalized_pbft_block(storage: &Storage, period: u64) -> H256 {
        write_mapped_pbft_block(storage, period, period)
    }

    fn write_mapped_pbft_block(
        storage: &Storage,
        embedded_period: u64,
        mapped_period: u64,
    ) -> H256 {
        let mut block = RlpStream::new_list(8);
        block.append(&H256::zero());
        block.append(&H256::from([0x77; 32]));
        block.begin_list(0);
        block.begin_list(0);
        block.append(&embedded_period);
        block.append(&0_u64);
        block.append(&0_u64);
        block.append(&Vec::<u8>::new());
        let block = block.out().to_vec();
        let block_hash = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block))
            .unwrap()
            .block_hash;
        let mut period_data = RlpStream::new_list(4);
        period_data.append_raw(&block, 1);
        period_data.begin_list(0);
        period_data.begin_list(0);
        period_data.begin_list(0);
        storage
            .period()
            .write(mapped_period, &period_data.out())
            .unwrap();
        storage
            .period()
            .write_pbft_period(block_hash, mapped_period)
            .unwrap();
        block_hash
    }

    fn validation(vote_rlp: &[u8]) -> PbftCanonicalVoteValidation {
        validate_canonical_pbft_vote(
            vote_rlp,
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
        .unwrap()
    }

    fn flags() -> PbftVoteEventFactFlags {
        PbftVoteEventFactFlags {
            vote_already_known: false,
            carries_proposed_block: true,
            valid_stale_reward_vote: false,
        }
    }

    fn stale_reward_flags() -> PbftVoteEventFactFlags {
        PbftVoteEventFactFlags {
            valid_stale_reward_vote: true,
            ..flags()
        }
    }

    fn context(threshold: Option<u64>) -> PbftVoteProgressContext {
        PbftVoteProgressContext {
            current_period: 12,
            current_round: 2,
            max_future_period_delta: 0,
            two_t_plus_one_threshold: threshold,
            require_proposed_block_sidecar: false,
            slashing_enabled: true,
        }
    }

    fn threshold_fact(has_total_dpos_votes_count: bool) -> PbftTwoTPlusOneThresholdFact {
        PbftTwoTPlusOneThresholdFact {
            pbft_period: 12,
            vote_type: PbftVoteType::Cert,
            current_pbft_chain_size: 12,
            committee_size: 100,
            number_of_proposers: 20,
            has_total_dpos_votes_count,
            total_dpos_votes_count: if has_total_dpos_votes_count { 100 } else { 0 },
            future_dpos_state: false,
            unknown_error: false,
        }
    }

    #[test]
    fn runtime_admits_vote_and_retains_payloads() {
        let rlp = vote_rlp([1; 32], 3);
        let validation = validation(&rlp);
        let mut runtime = PbftVoteAdmissionRuntime::new();

        let outcome = runtime
            .admit_validated_vote(&rlp, &validation, flags(), context(Some(50)))
            .unwrap();

        assert!(
            outcome
                .execution
                .unwrap()
                .pipeline_step
                .progress_plan
                .status
                == crate::pbft_vote_progress::PbftVoteProgressStatus::Accepted
        );
        assert!(outcome.storage_vote.is_some());
        assert!(runtime.weighted_payload(validation.vote_hash).is_some());
        assert!(runtime.slashing_payload(validation.vote_hash).is_some());
        assert!(runtime.replay_contains(validation.vote_hash));

        let payloads = runtime.weighted_payloads();
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].hash, validation.vote_hash);
        assert!(!payloads[0].vote_rlp.is_empty());
    }

    #[test]
    fn transactional_admission_publishes_no_write_without_calling_persistence() {
        let rlp = vote_rlp([0x31; 32], 3);
        let validation = validation(&rlp);
        let mut runtime = PbftVoteAdmissionRuntime::new();

        let transaction = runtime
            .admit_validated_vote_transactional(
                &rlp,
                &validation,
                flags(),
                context(Some(50)),
                |_| panic!("no-write admission must not invoke persistence"),
            )
            .unwrap();

        assert!(transaction.transition_published);
        assert!(!transaction.persistence_required);
        assert_eq!(
            transaction.persistence_status,
            PbftVoteAdmissionPersistenceStatus::NotRequired
        );
        assert_eq!(transaction.persistence_applied_writes, 0);
        assert!(runtime.replay_contains(validation.vote_hash));
        assert!(runtime.weighted_payload(validation.vote_hash).is_some());
    }

    #[test]
    fn transactional_admission_rejection_restores_fifo_state_and_allows_retry() {
        let mut runtime = PbftVoteAdmissionRuntime::new();
        let first = vote_rlp([0x32; 32], 3);
        let first_validation = validation(&first);
        runtime
            .admit_validated_vote(&first, &first_validation, flags(), context(Some(80)))
            .unwrap();
        runtime.reward_vote_cursor = Some(RewardVoteCursor {
            period: 12,
            round: 2,
            step: 3,
            block_hash: H256::from([0x32; 32]),
        });
        runtime.replay_cache = PbftVoteReplayCache::new(2, 1);
        let oldest = H256::from_low_u64_be(101);
        let newest = H256::from_low_u64_be(102);
        assert!(runtime.replay_insert(oldest));
        assert!(runtime.replay_insert(newest));

        let rlp = vote_rlp_from_secret([0x32; 32], 3, NODE_SECRET_TWO);
        let validation = validation(&rlp);
        let mut persistence_context = context(Some(80));
        persistence_context.current_period = 13;
        let rejected = runtime
            .admit_validated_vote_transactional(
                &rlp,
                &validation,
                stale_reward_flags(),
                persistence_context,
                |write| {
                    assert!(write.extra_reward_vote.is_some());
                    Ok(PbftVotePersistenceResult {
                        status: PbftVotePersistenceStatus::Rejected,
                        applied_writes: 0,
                        error_code: "TEST_PERSISTENCE_REJECTED".to_owned(),
                    })
                },
            )
            .unwrap();

        assert!(rejected.persistence_required);
        assert_eq!(
            rejected.persistence_status,
            PbftVoteAdmissionPersistenceStatus::Rejected
        );
        assert!(!rejected.transition_published);
        assert_eq!(rejected.persistence_applied_writes, 0);
        assert_eq!(rejected.persistence_error_code, "TEST_PERSISTENCE_REJECTED");
        assert!(runtime.replay_contains(oldest));
        assert!(runtime.replay_contains(newest));
        assert!(!runtime.replay_contains(validation.vote_hash));
        assert!(runtime.weighted_payload(validation.vote_hash).is_none());
        assert_eq!(runtime.verified_votes().size(), 1);

        let retried = runtime
            .admit_validated_vote_transactional(
                &rlp,
                &validation,
                stale_reward_flags(),
                persistence_context,
                |write| {
                    assert!(write.extra_reward_vote.is_some());
                    Ok(PbftVotePersistenceResult {
                        status: PbftVotePersistenceStatus::Applied,
                        applied_writes: 1,
                        error_code: String::new(),
                    })
                },
            )
            .unwrap();

        assert!(retried.transition_published);
        assert_eq!(
            retried.persistence_status,
            PbftVoteAdmissionPersistenceStatus::Applied
        );
        assert_eq!(retried.persistence_applied_writes, 1);
        assert!(!runtime.replay_contains(oldest));
        assert!(runtime.replay_contains(newest));
        assert!(runtime.replay_contains(validation.vote_hash));
        assert!(runtime.weighted_payload(validation.vote_hash).is_some());
        assert_eq!(runtime.verified_votes().size(), 2);
    }

    #[test]
    fn transactional_admission_operational_failure_is_typed_and_restores_state() {
        let first = vote_rlp([0x33; 32], 3);
        let first_validation = validation(&first);
        let rlp = vote_rlp_from_secret([0x33; 32], 3, NODE_SECRET_TWO);
        let validation = validation(&rlp);
        let mut runtime = PbftVoteAdmissionRuntime::new();
        runtime
            .admit_validated_vote(&first, &first_validation, flags(), context(Some(80)))
            .unwrap();
        runtime.reward_vote_cursor = Some(RewardVoteCursor {
            period: 12,
            round: 2,
            step: 3,
            block_hash: H256::from([0x33; 32]),
        });
        let mut persistence_context = context(Some(80));
        persistence_context.current_period = 13;

        let transaction = runtime
            .admit_validated_vote_transactional(
                &rlp,
                &validation,
                stale_reward_flags(),
                persistence_context,
                |_| Err(anyhow::anyhow!("injected storage failure")),
            )
            .unwrap();

        assert!(transaction.persistence_required);
        assert_eq!(
            transaction.persistence_status,
            PbftVoteAdmissionPersistenceStatus::Rejected
        );
        assert!(!transaction.transition_published);
        assert_eq!(
            transaction.persistence_error_code,
            "PBFT_VOTE_PERSIST_STORAGE_OR_LOCK_FAILURE"
        );
        assert!(!runtime.replay_contains(validation.vote_hash));
        assert!(runtime.weighted_payload(validation.vote_hash).is_none());
        assert_eq!(runtime.verified_votes().size(), 1);
    }

    #[test]
    fn runtime_owns_replay_cache_and_thresholds() {
        let mut runtime = PbftVoteAdmissionRuntime::new_with_replay_cache(1, 1);
        let first_hash = H256::from_low_u64_be(1);
        let second_hash = H256::from_low_u64_be(2);

        assert!(runtime.replay_insert(first_hash));
        assert!(!runtime.replay_insert(first_hash));
        assert!(runtime.replay_contains(first_hash));
        assert!(runtime.replay_insert(second_hash));
        assert!(!runtime.replay_contains(first_hash));
        assert!(runtime.replay_contains(second_hash));

        let plan = runtime.plan_two_t_plus_one_threshold(threshold_fact(true));
        assert!(plan.has_threshold);

        let cached = runtime.plan_two_t_plus_one_threshold(threshold_fact(false));
        assert!(cached.cache_hit);
        assert_eq!(cached.threshold, plan.threshold);
    }

    #[test]
    fn runtime_cleanup_plan_counts_and_directly_prunes_stale_payloads() {
        let old = vote_rlp_for([0x41; 32], 11, 2, 3, NODE_SECRET);
        let kept = vote_rlp_for([0x42; 32], 12, 2, 3, NODE_SECRET_TWO);
        let old_validation = validation(&old);
        let kept_validation = validation(&kept);
        let mut runtime = PbftVoteAdmissionRuntime::new();

        let mut old_context = context(Some(100));
        old_context.current_period = 11;
        runtime
            .admit_validated_vote(&old, &old_validation, flags(), old_context)
            .unwrap();
        runtime
            .admit_validated_vote(&kept, &kept_validation, flags(), context(Some(100)))
            .unwrap();

        let plan = runtime.plan_cleanup_votes_by_period(12);
        assert_eq!(plan.first_period_to_keep(), 12);
        assert_eq!(plan.periods_removed(), 1);
        assert_eq!(plan.votes_removed(), 1);
        assert_eq!(plan.payloads_removed(), 1);
        assert!(runtime.weighted_payload(old_validation.vote_hash).is_some());
        assert!(
            runtime
                .weighted_payload(kept_validation.vote_hash)
                .is_some()
        );

        runtime.apply_cleanup_votes_by_period(&plan);
        assert!(runtime.weighted_payload(old_validation.vote_hash).is_none());
        assert!(
            runtime
                .weighted_payload(kept_validation.vote_hash)
                .is_some()
        );
        assert_eq!(runtime.verified_votes().size(), 1);
    }

    #[test]
    fn runtime_builds_two_t_plus_one_bundle_from_retained_payloads() {
        let mut runtime = PbftVoteAdmissionRuntime::new();
        let first = vote_rlp([2; 32], 3);
        let second = vote_rlp_from_secret([2; 32], 3, NODE_SECRET_TWO);
        let first_validation = validation(&first);
        let second_validation = validation(&second);

        runtime
            .admit_validated_vote(&first, &first_validation, flags(), context(Some(80)))
            .unwrap();
        let outcome = runtime
            .admit_validated_vote(&second, &second_validation, flags(), context(Some(80)))
            .unwrap();

        let bundle = outcome.two_t_plus_one_bundle.unwrap();
        assert_eq!(bundle.votes_count, 2);
        assert!(!bundle.votes_bundle_rlp.is_empty());

        let payloads = runtime
            .two_t_plus_one_weighted_payloads(12, 2, TwoTPlusOneVotedBlockType::CertVotedBlock)
            .unwrap()
            .expect("cert mapping is retained");
        assert_eq!(payloads.len(), 2);
        let payload_hashes = payloads
            .into_iter()
            .map(|payload| payload.hash)
            .collect::<Vec<_>>();
        assert!(payload_hashes.contains(&first_validation.vote_hash));
        assert!(payload_hashes.contains(&second_validation.vote_hash));
        assert!(
            runtime
                .two_t_plus_one_weighted_payloads(12, 3, TwoTPlusOneVotedBlockType::CertVotedBlock)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn service_persists_vote_progress_from_native_payload_identities() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustaxa_vote_progress_identity_{nonce}"));
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let mut runtime = PbftVoteAdmissionRuntime::new();
        let first = vote_rlp([2; 32], 3);
        let second = vote_rlp_from_secret([2; 32], 3, NODE_SECRET_TWO);
        let first_validation = validation(&first);
        let second_validation = validation(&second);
        runtime
            .admit_validated_vote(&first, &first_validation, flags(), context(Some(80)))
            .unwrap();
        runtime
            .admit_validated_vote(&second, &second_validation, flags(), context(Some(80)))
            .unwrap();
        let service = PbftVerifiedVotesService::from_runtime(storage.clone(), runtime);

        let mismatch = service
            .verified_votes_persist_pbft_vote_progress(PbftVerifiedVoteProgressPersistenceWrite {
                extra_reward_vote_hash: Some(first_validation.vote_hash),
                two_t_plus_one_bundle: Some(PbftVerifiedVoteProgressBundle {
                    kind: TwoTPlusOneVotedBlockType::CertVotedBlock,
                    period: 12,
                    round: 2,
                    step: 4,
                    block_hash: H256::from([2; 32]),
                }),
            })
            .unwrap_err();
        assert!(
            mismatch
                .to_string()
                .contains("PBFT_VOTE_PROGRESS_TWO_T_PLUS_ONE_IDENTITY_MISMATCH")
        );
        assert!(storage.pbft().reward_votes_rlp().unwrap().is_empty());
        assert!(
            storage
                .pbft()
                .all_two_t_plus_one_votes_rlp()
                .unwrap()
                .is_empty()
        );

        let result = service
            .verified_votes_persist_pbft_vote_progress(PbftVerifiedVoteProgressPersistenceWrite {
                extra_reward_vote_hash: Some(first_validation.vote_hash),
                two_t_plus_one_bundle: Some(PbftVerifiedVoteProgressBundle {
                    kind: TwoTPlusOneVotedBlockType::CertVotedBlock,
                    period: 12,
                    round: 2,
                    step: 3,
                    block_hash: H256::from([2; 32]),
                }),
            })
            .unwrap();

        assert_eq!(result.status, PbftVotePersistenceStatus::Applied);
        assert_eq!(result.applied_writes, 2);
        assert_eq!(storage.pbft().reward_votes_rlp().unwrap().len(), 1);
        assert_eq!(
            storage.pbft().all_two_t_plus_one_votes_rlp().unwrap().len(),
            2
        );

        drop(service);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn runtime_selects_reward_payloads_from_reverse_round_in_requested_order() {
        let mut runtime = PbftVoteAdmissionRuntime::new();
        let reward_block = [6; 32];
        let preferred = VerifiedVote::new(
            H256::from_low_u64_be(1),
            H256::from(reward_block),
            H160::from_low_u64_be(1),
            12,
            1,
            3,
            PbftVoteType::Cert,
            1,
        )
        .unwrap();
        runtime
            .verified_votes_mut()
            .add_verified_vote(preferred, None)
            .unwrap();
        runtime.reward_vote_cursor = Some(RewardVoteCursor {
            period: 12,
            round: 1,
            step: 3,
            block_hash: reward_block.into(),
        });

        let first = vote_rlp_for(reward_block, 12, 2, 3, NODE_SECRET);
        let second = vote_rlp_for(reward_block, 12, 2, 3, NODE_SECRET_TWO);
        let first_validation = validation(&first);
        let second_validation = validation(&second);
        runtime
            .admit_validated_vote(&first, &first_validation, flags(), context(Some(80)))
            .unwrap();
        runtime
            .admit_validated_vote(&second, &second_validation, flags(), context(Some(80)))
            .unwrap();

        let selection = runtime
            .select_reward_vote_payloads(
                13,
                vec![second_validation.vote_hash, first_validation.vote_hash],
            )
            .unwrap();

        assert!(selection.accepted);
        assert_eq!(selection.selected_round, 2);
        assert_eq!(
            selection.selected_vote_hashes,
            vec![second_validation.vote_hash, first_validation.vote_hash]
        );
        assert_eq!(selection.selected_records.len(), 2);
        assert_eq!(
            selection.selected_records[0].hash,
            second_validation.vote_hash
        );
        assert_eq!(
            selection.selected_records[1].hash,
            first_validation.vote_hash
        );
    }

    #[test]
    fn runtime_errors_when_selected_reward_payload_is_missing() {
        let mut runtime = PbftVoteAdmissionRuntime::new();
        let vote_hash = H256::from_low_u64_be(77);
        let reward_block = H256::from_low_u64_be(88);
        let metadata_only_vote = VerifiedVote::new(
            vote_hash,
            reward_block,
            H160::from_low_u64_be(9),
            12,
            1,
            3,
            PbftVoteType::Cert,
            1,
        )
        .unwrap();
        runtime
            .verified_votes_mut()
            .add_verified_vote(metadata_only_vote, None)
            .unwrap();
        runtime.reward_vote_cursor = Some(RewardVoteCursor {
            period: 12,
            round: 1,
            step: 3,
            block_hash: reward_block,
        });

        let err = runtime
            .select_reward_vote_payloads(13, vec![vote_hash])
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("missing retained weighted payload")
        );
    }

    #[test]
    fn runtime_returns_slashing_payloads_for_conflict() {
        let mut runtime = PbftVoteAdmissionRuntime::new();
        let first = vote_rlp([4; 32], 3);
        let second = vote_rlp([5; 32], 3);
        let first_validation = validation(&first);
        let second_validation = validation(&second);

        runtime
            .admit_validated_vote(&first, &first_validation, flags(), context(Some(100)))
            .unwrap();
        let outcome = runtime
            .admit_validated_vote(&second, &second_validation, flags(), context(Some(100)))
            .unwrap();

        let slashing = outcome.slashing_payloads.unwrap();
        assert_eq!(slashing.conflicting.hash, first_validation.vote_hash);
        assert_eq!(slashing.incoming.hash, second_validation.vote_hash);
    }

    #[test]
    fn runtime_restores_and_deduplicates_persisted_vote_families() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustaxa_verified_votes_restore_{nonce}"));
        let storage = Storage::new(Config::new(path.clone())).unwrap();
        let canonical = vote_rlp([8; 32], 3);
        let weighted = build_weighted_pbft_vote_payload(&canonical, 7).unwrap();
        let bundle = build_weighted_pbft_vote_bundle(std::slice::from_ref(&weighted)).unwrap();
        storage
            .pbft()
            .replace_two_t_plus_one_votes(1, &bundle)
            .unwrap();
        storage
            .pbft()
            .write_own_verified_vote(weighted.hash, &weighted.vote_rlp)
            .unwrap();
        storage
            .pbft()
            .write_extra_reward_vote(weighted.hash, &weighted.vote_rlp)
            .unwrap();
        let mut cursor_batch = storage.create_write_batch();
        storage
            .pbft()
            .write_finalized_reward_vote_cursor_in_batch(
                &mut cursor_batch,
                rustaxa_storage::StoredFinalizedRewardVoteCursor {
                    period: 12,
                    round: 2,
                    step: 3,
                    block_hash: H256::from([8; 32]),
                    votes_bundle_rlp: bundle.clone(),
                },
            )
            .unwrap();
        storage
            .commit_write_batch_with_sync(cursor_batch, false)
            .unwrap();

        let runtime = PbftVoteAdmissionRuntime::restore_from_storage(&storage).unwrap();

        assert_eq!(runtime.verified_votes().size(), 1);
        assert_eq!(runtime.weighted_payloads(), vec![weighted.clone()]);
        assert!(runtime.replay_contains(weighted.hash));
        assert_eq!(
            runtime.reward_vote_cursor(),
            Some(RewardVoteCursor {
                period: 12,
                round: 2,
                step: 3,
                block_hash: H256::from([8; 32]),
            })
        );

        drop(runtime);
        drop(storage);
        let reopened = Storage::new(Config::new(path.clone())).unwrap();
        let restored = PbftVoteAdmissionRuntime::restore_from_storage(&reopened).unwrap();
        assert_eq!(
            restored.reward_vote_cursor(),
            Some(RewardVoteCursor {
                period: 12,
                round: 2,
                step: 3,
                block_hash: H256::from([8; 32]),
            })
        );
        assert_eq!(
            restored.current_reward_vote_payloads().unwrap(),
            vec![weighted]
        );
        drop(reopened);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn restart_keeps_finalized_reward_cursor_after_newer_cert_progress_overwrite() {
        use crate::pbft_vote_storage::{
            PbftTwoTPlusOneVoteBundle, PbftVoteProgressPersistenceWrite, persist_pbft_vote_progress,
        };

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rustaxa_finalized_reward_cursor_crash_window_{nonce}"
        ));
        let storage = Storage::new(Config::new(path.clone())).unwrap();
        let finalized_block = write_finalized_pbft_block(&storage, 12);
        let finalized_vote = build_weighted_pbft_vote_payload(
            &vote_rlp_for(finalized_block.0, 12, 2, 3, NODE_SECRET),
            7,
        )
        .unwrap();
        let finalized_bundle =
            build_weighted_pbft_vote_bundle(std::slice::from_ref(&finalized_vote)).unwrap();
        storage
            .pbft()
            .replace_two_t_plus_one_votes(1, &finalized_bundle)
            .unwrap();
        write_legacy_pbft_head(&storage, 12, finalized_block);
        drop(storage);

        let migrated_storage = Storage::new(Config::new(path.clone())).unwrap();
        let migrated = PbftVoteAdmissionRuntime::restore_from_storage(&migrated_storage).unwrap();
        assert_eq!(
            migrated.reward_vote_cursor(),
            Some(RewardVoteCursor {
                period: 12,
                round: 2,
                step: 3,
                block_hash: finalized_block,
            })
        );
        assert_eq!(
            migrated.current_reward_vote_payloads().unwrap(),
            vec![finalized_vote.clone()]
        );
        assert!(
            migrated_storage
                .pbft()
                .finalized_reward_vote_cursor()
                .unwrap()
                .is_some()
        );
        drop(migrated);
        drop(migrated_storage);

        let storage = Storage::new(Config::new(path.clone())).unwrap();

        let newer_vote =
            build_weighted_pbft_vote_payload(&vote_rlp_for([22; 32], 13, 1, 3, NODE_SECRET_TWO), 7)
                .unwrap();
        let newer_bundle =
            build_weighted_pbft_vote_bundle(std::slice::from_ref(&newer_vote)).unwrap();
        let persisted = persist_pbft_vote_progress(
            &storage,
            PbftVoteProgressPersistenceWrite {
                extra_reward_vote: None,
                two_t_plus_one_bundle: Some(PbftTwoTPlusOneVoteBundle {
                    kind: 1,
                    period: 13,
                    round: 1,
                    step: 3,
                    block_hash: H256::from([22; 32]),
                    votes_bundle_rlp: newer_bundle,
                }),
            },
        )
        .unwrap();
        assert_eq!(persisted.status.as_u8(), 0);

        drop(storage);
        let reopened = Storage::new(Config::new(path.clone())).unwrap();
        let restored = PbftVoteAdmissionRuntime::restore_from_storage(&reopened).unwrap();
        assert_eq!(
            restored.reward_vote_cursor(),
            Some(RewardVoteCursor {
                period: 12,
                round: 2,
                step: 3,
                block_hash: finalized_block,
            })
        );
        assert_eq!(
            restored.current_reward_vote_payloads().unwrap(),
            vec![finalized_vote.clone()]
        );
        let selection = restored
            .select_reward_vote_payloads(13, vec![finalized_vote.hash])
            .unwrap();
        assert!(selection.accepted);
        assert_eq!(selection.selected_records, vec![finalized_vote]);

        drop(restored);
        drop(reopened);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn legacy_reward_cursor_bootstrap_rejects_newer_unfinalized_cert_bundle() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("rustaxa_legacy_reward_cursor_newer_cert_{nonce}"));
        let storage = Storage::new(Config::new(path.clone())).unwrap();
        let finalized_block = write_finalized_pbft_block(&storage, 12);
        write_legacy_pbft_head(&storage, 12, finalized_block);
        let newer =
            build_weighted_pbft_vote_payload(&vote_rlp_for([23; 32], 13, 1, 3, NODE_SECRET), 7)
                .unwrap();
        storage
            .pbft()
            .replace_two_t_plus_one_votes(1, &build_weighted_pbft_vote_bundle(&[newer]).unwrap())
            .unwrap();

        let error = PbftVoteAdmissionRuntime::restore_from_storage(&storage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not match finalized PBFT head"));
        assert!(
            storage
                .pbft()
                .finalized_reward_vote_cursor()
                .unwrap()
                .is_none()
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn legacy_reward_cursor_bootstrap_rejects_embedded_pbft_period_mismatch() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rustaxa_legacy_reward_cursor_embedded_period_{nonce}"
        ));
        let storage = Storage::new(Config::new(path.clone())).unwrap();
        let block_hash = write_mapped_pbft_block(&storage, 11, 12);
        write_legacy_pbft_head(&storage, 12, block_hash);
        let vote =
            build_weighted_pbft_vote_payload(&vote_rlp_for(block_hash.0, 12, 2, 3, NODE_SECRET), 7)
                .unwrap();
        storage
            .pbft()
            .replace_two_t_plus_one_votes(1, &build_weighted_pbft_vote_bundle(&[vote]).unwrap())
            .unwrap();

        let error = PbftVoteAdmissionRuntime::restore_from_storage(&storage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("embedded period mismatch"));
        assert!(
            storage
                .pbft()
                .finalized_reward_vote_cursor()
                .unwrap()
                .is_none()
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn legacy_reward_cursor_bootstrap_rejects_nonempty_head_without_cert_bundle() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("rustaxa_legacy_reward_cursor_missing_cert_{nonce}"));
        let storage = Storage::new(Config::new(path.clone())).unwrap();
        let finalized_block = write_finalized_pbft_block(&storage, 12);
        write_legacy_pbft_head(&storage, 12, finalized_block);

        let error = PbftVoteAdmissionRuntime::restore_from_storage(&storage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("finalized PBFT head has no cert bundle"));

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn legacy_reward_cursor_bootstrap_rejects_malformed_cert_bundle() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rustaxa_legacy_reward_cursor_malformed_cert_{nonce}"
        ));
        let storage = Storage::new(Config::new(path.clone())).unwrap();
        let finalized_block = write_finalized_pbft_block(&storage, 12);
        write_legacy_pbft_head(&storage, 12, finalized_block);
        storage
            .pbft()
            .replace_two_t_plus_one_votes(1, &[0x01])
            .unwrap();

        assert!(PbftVoteAdmissionRuntime::restore_from_storage(&storage).is_err());
        assert!(
            storage
                .pbft()
                .finalized_reward_vote_cursor()
                .unwrap()
                .is_none()
        );

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn runtime_rejects_malformed_persisted_vote() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustaxa_verified_votes_bad_restore_{nonce}"));
        let storage = Storage::new(Config::new(path.clone())).unwrap();
        storage
            .pbft()
            .write_own_verified_vote(H256::from_low_u64_be(1), &[0x01])
            .unwrap();

        assert!(PbftVoteAdmissionRuntime::restore_from_storage(&storage).is_err());

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn runtime_rejects_own_vote_payload_whose_hash_does_not_match_key() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rustaxa_verified_votes_hash_mismatch_restore_{nonce}"
        ));
        let storage = Storage::new(Config::new(path.clone())).unwrap();
        let weighted = build_weighted_pbft_vote_payload(&vote_rlp([9; 32], 3), 7).unwrap();
        let wrong_hash = H256::from_low_u64_be(99);
        assert_ne!(wrong_hash, weighted.hash);
        storage
            .pbft()
            .write_own_verified_vote(wrong_hash, &weighted.vote_rlp)
            .unwrap();

        assert!(PbftVoteAdmissionRuntime::restore_from_storage(&storage).is_err());

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn runtime_rejects_extra_reward_payload_whose_hash_does_not_match_key() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rustaxa_extra_reward_hash_mismatch_restore_{nonce}"
        ));
        let storage = Storage::new(Config::new(path.clone())).unwrap();
        let weighted = build_weighted_pbft_vote_payload(&vote_rlp([10; 32], 3), 7).unwrap();
        let wrong_hash = H256::from_low_u64_be(100);
        assert_ne!(wrong_hash, weighted.hash);
        storage
            .pbft()
            .write_extra_reward_vote(wrong_hash, &weighted.vote_rlp)
            .unwrap();

        assert!(PbftVoteAdmissionRuntime::restore_from_storage(&storage).is_err());

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn reward_cursor_commit_is_generation_bound_idempotent_and_retryable() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustaxa_reward_cursor_commit_{nonce}"));
        let storage = Storage::new(Config::new(path.clone())).unwrap();
        let weighted = build_weighted_pbft_vote_payload(&vote_rlp([11; 32], 3), 7).unwrap();
        let bundle = build_weighted_pbft_vote_bundle(std::slice::from_ref(&weighted)).unwrap();
        storage
            .pbft()
            .replace_two_t_plus_one_votes(1, &bundle)
            .unwrap();
        let cursor = RewardVoteCursor {
            period: 12,
            round: 2,
            step: 3,
            block_hash: H256::from([11; 32]),
        };
        let mut initial_cursor_batch = storage.create_write_batch();
        storage
            .pbft()
            .write_finalized_reward_vote_cursor_in_batch(
                &mut initial_cursor_batch,
                rustaxa_storage::StoredFinalizedRewardVoteCursor {
                    period: cursor.period,
                    round: cursor.round,
                    step: cursor.step,
                    block_hash: cursor.block_hash,
                    votes_bundle_rlp: bundle.clone(),
                },
            )
            .unwrap();
        storage
            .commit_write_batch_with_sync(initial_cursor_batch, false)
            .unwrap();
        let mut runtime = PbftVoteAdmissionRuntime::restore_from_storage(&storage).unwrap();
        assert_eq!(runtime.reward_vote_cursor(), Some(cursor));
        runtime.reward_vote_cursor = None;

        let rejected = runtime
            .commit_reward_vote_cursor(&storage, cursor, 1)
            .unwrap();
        assert_eq!(rejected.status, RewardVoteCursorCommitStatus::Rejected);
        assert!(runtime.reward_vote_cursor().is_none());

        let generation = {
            let guard = storage.lock_extra_reward_votes().unwrap();
            let mut batch = storage.create_write_batch();
            storage
                .pbft()
                .write_finalized_reward_vote_cursor_in_batch(
                    &mut batch,
                    rustaxa_storage::StoredFinalizedRewardVoteCursor {
                        period: cursor.period,
                        round: cursor.round,
                        step: cursor.step,
                        block_hash: cursor.block_hash,
                        votes_bundle_rlp: bundle.clone(),
                    },
                )
                .unwrap();
            guard.commit_reset_batch(batch, false).unwrap()
        };
        let mut corrupt_cursor_batch = storage.create_write_batch();
        storage
            .pbft()
            .write_finalized_reward_vote_cursor_in_batch(
                &mut corrupt_cursor_batch,
                rustaxa_storage::StoredFinalizedRewardVoteCursor {
                    period: cursor.period,
                    round: cursor.round,
                    step: cursor.step,
                    block_hash: cursor.block_hash,
                    votes_bundle_rlp: vec![0xc1, 0x01],
                },
            )
            .unwrap();
        storage
            .commit_write_batch_with_sync(corrupt_cursor_batch, false)
            .unwrap();
        let durable_reject = runtime
            .commit_reward_vote_cursor(&storage, cursor, generation)
            .unwrap();
        assert_eq!(
            durable_reject.status,
            RewardVoteCursorCommitStatus::Rejected
        );
        assert!(runtime.reward_vote_cursor().is_none());

        let mut restore_cursor_batch = storage.create_write_batch();
        storage
            .pbft()
            .write_finalized_reward_vote_cursor_in_batch(
                &mut restore_cursor_batch,
                rustaxa_storage::StoredFinalizedRewardVoteCursor {
                    period: cursor.period,
                    round: cursor.round,
                    step: cursor.step,
                    block_hash: cursor.block_hash,
                    votes_bundle_rlp: bundle.clone(),
                },
            )
            .unwrap();
        storage
            .commit_write_batch_with_sync(restore_cursor_batch, false)
            .unwrap();
        let applied = runtime
            .commit_reward_vote_cursor(&storage, cursor, generation)
            .unwrap();
        assert_eq!(applied.status, RewardVoteCursorCommitStatus::Applied);
        let repeated = runtime
            .commit_reward_vote_cursor(&storage, cursor, generation)
            .unwrap();
        assert_eq!(
            repeated.status,
            RewardVoteCursorCommitStatus::AlreadyCurrent
        );
        assert_eq!(runtime.reward_vote_cursor(), Some(cursor));
        let newer_replay_generation = {
            let guard = storage.lock_extra_reward_votes().unwrap();
            guard
                .commit_reset_batch(storage.create_write_batch(), false)
                .unwrap()
        };
        let newer_replay = runtime
            .commit_reward_vote_cursor(&storage, cursor, newer_replay_generation)
            .unwrap();
        assert_eq!(
            newer_replay.status,
            RewardVoteCursorCommitStatus::AlreadyCurrent
        );
        assert_eq!(
            runtime.reward_vote_cursor_reset_generation,
            newer_replay_generation
        );

        let install_candidate = |runtime: &mut PbftVoteAdmissionRuntime,
                                 storage: &Storage,
                                 period,
                                 round,
                                 block_hash: [u8; 32]| {
            let canonical = vote_rlp_for(block_hash, period, round, 3, NODE_SECRET_TWO);
            let inspection = inspect_restored_weighted_vote(
                &build_weighted_pbft_vote_payload(&canonical, 40)
                    .unwrap()
                    .vote_rlp,
            )
            .unwrap();
            let weighted = build_weighted_pbft_vote_payload(&canonical, 40).unwrap();
            runtime
                .verified_votes_mut()
                .add_verified_vote(
                    VerifiedVote::new(
                        inspection.vote_hash,
                        inspection.block_hash,
                        inspection.recovered_voter,
                        inspection.period,
                        inspection.round,
                        inspection.step,
                        inspection.vote_type,
                        inspection.embedded_weight,
                    )
                    .unwrap(),
                    None,
                )
                .unwrap();
            runtime.payloads.insert(
                weighted.hash,
                PbftVoteRuntimePayload {
                    slashing: build_slashing_pbft_vote_payload(&canonical).unwrap(),
                    weighted: weighted.clone(),
                },
            );
            let inserted = runtime
                .verified_votes_mut()
                .insert_two_t_plus_one_voted_block(
                    period,
                    round,
                    TwoTPlusOneVotedBlockType::CertVotedBlock,
                    block_hash.into(),
                    3,
                );
            assert!(inserted.round_found);
            let bundle = build_weighted_pbft_vote_bundle(&[weighted]).unwrap();
            storage
                .pbft()
                .replace_two_t_plus_one_votes(1, &bundle)
                .unwrap();
            let generation = {
                let guard = storage.lock_extra_reward_votes().unwrap();
                let mut batch = storage.create_write_batch();
                storage
                    .pbft()
                    .write_finalized_reward_vote_cursor_in_batch(
                        &mut batch,
                        rustaxa_storage::StoredFinalizedRewardVoteCursor {
                            period,
                            round,
                            step: 3,
                            block_hash: block_hash.into(),
                            votes_bundle_rlp: bundle,
                        },
                    )
                    .unwrap();
                guard.commit_reset_batch(batch, false).unwrap()
            };
            (
                RewardVoteCursor {
                    period,
                    round,
                    step: 3,
                    block_hash: block_hash.into(),
                },
                generation,
            )
        };
        let (same_period_conflict, conflict_generation) =
            install_candidate(&mut runtime, &storage, 12, 3, [12; 32]);
        let conflict = runtime
            .commit_reward_vote_cursor(&storage, same_period_conflict, conflict_generation)
            .unwrap();
        assert_eq!(conflict.status, RewardVoteCursorCommitStatus::Rejected);
        assert_eq!(conflict.error_code, "PBFT_REWARD_CURSOR_NOT_MONOTONIC");
        assert_eq!(runtime.reward_vote_cursor(), Some(cursor));

        let (newer, newer_generation) = install_candidate(&mut runtime, &storage, 13, 1, [13; 32]);
        let advanced = runtime
            .commit_reward_vote_cursor(&storage, newer, newer_generation)
            .unwrap();
        assert_eq!(advanced.status, RewardVoteCursorCommitStatus::Applied);
        assert_eq!(runtime.reward_vote_cursor(), Some(newer));

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn stale_reward_eligibility_uses_cursor_and_saturating_round_window() {
        let persists_extra_reward =
            |cursor: Option<RewardVoteCursor>,
             vote: Vec<u8>,
             mutate_validation: fn(&mut PbftCanonicalVoteValidation)| {
                let mut runtime = PbftVoteAdmissionRuntime::new();
                runtime.reward_vote_cursor = cursor;
                let mut vote_validation = validation(&vote);
                mutate_validation(&mut vote_validation);
                let outcome = runtime
                    .admit_validated_vote(&vote, &vote_validation, flags(), context(None))
                    .unwrap();
                outcome.execution.as_ref().is_some_and(|execution| {
                execution
                    .pipeline_step
                    .progress_plan
                    .intents
                    .iter()
                    .any(|intent| {
                        matches!(
                            intent,
                            crate::pbft_vote_progress::PbftVoteProgressIntent::PersistExtraRewardVote { .. }
                        )
                    })
            })
            };
        let unchanged = |_: &mut PbftCanonicalVoteValidation| {};
        let block_hash = [12; 32];
        let cursor = RewardVoteCursor {
            period: 11,
            round: 20,
            step: 3,
            block_hash: block_hash.into(),
        };

        assert!(!persists_extra_reward(
            None,
            vote_rlp_for(block_hash, 11, 20, 3, NODE_SECRET),
            unchanged,
        ));
        assert!(!persists_extra_reward(
            Some(cursor),
            vote_rlp_for(block_hash, 11, 20, 3, NODE_SECRET),
            |validation| validation.vote_type = PbftVoteType::Soft,
        ));
        assert!(!persists_extra_reward(
            Some(cursor),
            vote_rlp_for(block_hash, 10, 20, 3, NODE_SECRET),
            unchanged,
        ));
        assert!(!persists_extra_reward(
            Some(cursor),
            vote_rlp_for([13; 32], 11, 20, 3, NODE_SECRET),
            unchanged,
        ));
        assert!(persists_extra_reward(
            Some(cursor),
            vote_rlp_for(block_hash, 11, 120, 3, NODE_SECRET),
            unchanged,
        ));
        assert!(!persists_extra_reward(
            Some(cursor),
            vote_rlp_for(block_hash, 11, 121, 3, NODE_SECRET),
            unchanged,
        ));

        let saturating_cursor = RewardVoteCursor {
            period: 11,
            round: u64::MAX - 50,
            step: 3,
            block_hash: block_hash.into(),
        };
        assert!(persists_extra_reward(
            Some(saturating_cursor),
            vote_rlp_for(block_hash, 11, u64::MAX, 3, NODE_SECRET),
            unchanged,
        ));
    }

    #[test]
    fn reward_cursor_reset_generation_is_single_use_for_new_cursor() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustaxa_reward_cursor_single_use_{nonce}"));
        let storage = Storage::new(Config::new(path.clone())).unwrap();
        let first_weighted =
            build_weighted_pbft_vote_payload(&vote_rlp_for([21; 32], 12, 1, 3, NODE_SECRET), 7)
                .unwrap();
        let first_bundle =
            build_weighted_pbft_vote_bundle(std::slice::from_ref(&first_weighted)).unwrap();
        storage
            .pbft()
            .replace_two_t_plus_one_votes(1, &first_bundle)
            .unwrap();
        let first = RewardVoteCursor {
            period: 12,
            round: 1,
            step: 3,
            block_hash: [21; 32].into(),
        };
        let mut first_cursor_batch = storage.create_write_batch();
        storage
            .pbft()
            .write_finalized_reward_vote_cursor_in_batch(
                &mut first_cursor_batch,
                rustaxa_storage::StoredFinalizedRewardVoteCursor {
                    period: first.period,
                    round: first.round,
                    step: first.step,
                    block_hash: first.block_hash,
                    votes_bundle_rlp: first_bundle.clone(),
                },
            )
            .unwrap();
        storage
            .commit_write_batch_with_sync(first_cursor_batch, false)
            .unwrap();
        let mut runtime = PbftVoteAdmissionRuntime::restore_from_storage(&storage).unwrap();
        assert_eq!(runtime.reward_vote_cursor(), Some(first));
        runtime.reward_vote_cursor = None;
        let generation_one = {
            let guard = storage.lock_extra_reward_votes().unwrap();
            guard
                .commit_reset_batch(storage.create_write_batch(), false)
                .unwrap()
        };
        assert_eq!(
            runtime
                .commit_reward_vote_cursor(&storage, first, generation_one)
                .unwrap()
                .status,
            RewardVoteCursorCommitStatus::Applied
        );

        let canonical = vote_rlp_for([22; 32], 13, 1, 3, NODE_SECRET_TWO);
        let weighted = build_weighted_pbft_vote_payload(&canonical, 9).unwrap();
        let inspection = inspect_restored_weighted_vote(&weighted.vote_rlp).unwrap();
        runtime
            .verified_votes_mut()
            .add_verified_vote(
                VerifiedVote::new(
                    inspection.vote_hash,
                    inspection.block_hash,
                    inspection.recovered_voter,
                    inspection.period,
                    inspection.round,
                    inspection.step,
                    inspection.vote_type,
                    inspection.embedded_weight,
                )
                .unwrap(),
                None,
            )
            .unwrap();
        runtime.payloads.insert(
            weighted.hash,
            PbftVoteRuntimePayload {
                slashing: build_slashing_pbft_vote_payload(&canonical).unwrap(),
                weighted: weighted.clone(),
            },
        );
        assert!(
            runtime
                .verified_votes_mut()
                .insert_two_t_plus_one_voted_block(
                    13,
                    1,
                    TwoTPlusOneVotedBlockType::CertVotedBlock,
                    [22; 32].into(),
                    3,
                )
                .round_found
        );
        let second_bundle = build_weighted_pbft_vote_bundle(&[weighted]).unwrap();
        storage
            .pbft()
            .replace_two_t_plus_one_votes(1, &second_bundle)
            .unwrap();
        let second = RewardVoteCursor {
            period: 13,
            round: 1,
            step: 3,
            block_hash: [22; 32].into(),
        };

        let reused = runtime
            .commit_reward_vote_cursor(&storage, second, generation_one)
            .unwrap();
        assert_eq!(reused.status, RewardVoteCursorCommitStatus::Rejected);
        assert_eq!(
            reused.error_code,
            "PBFT_REWARD_CURSOR_RESET_GENERATION_CONSUMED"
        );
        assert_eq!(runtime.reward_vote_cursor(), Some(first));

        let generation_two = {
            let guard = storage.lock_extra_reward_votes().unwrap();
            let mut batch = storage.create_write_batch();
            storage
                .pbft()
                .write_finalized_reward_vote_cursor_in_batch(
                    &mut batch,
                    rustaxa_storage::StoredFinalizedRewardVoteCursor {
                        period: second.period,
                        round: second.round,
                        step: second.step,
                        block_hash: second.block_hash,
                        votes_bundle_rlp: second_bundle,
                    },
                )
                .unwrap();
            guard.commit_reset_batch(batch, false).unwrap()
        };
        assert_eq!(
            runtime
                .commit_reward_vote_cursor(&storage, second, generation_two)
                .unwrap()
                .status,
            RewardVoteCursorCommitStatus::Applied
        );
        assert_eq!(runtime.reward_vote_cursor(), Some(second));

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn verified_votes_service_owns_reward_reset_storage_and_live_publication() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustaxa_reward_reset_service_{nonce}"));
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let block_hash = [0x61; 32];
        let first = vote_rlp_from_secret(block_hash, 3, NODE_SECRET);
        let second = vote_rlp_from_secret(block_hash, 3, NODE_SECRET_TWO);
        let first_validation = validation(&first);
        let second_validation = validation(&second);
        let mut runtime = PbftVoteAdmissionRuntime::new();
        runtime
            .admit_validated_vote(&first, &first_validation, flags(), context(Some(80)))
            .unwrap();
        runtime
            .admit_validated_vote(&second, &second_validation, flags(), context(Some(80)))
            .unwrap();
        storage
            .pbft()
            .write_extra_reward_vote(H256::from_low_u64_be(701), &[0x01])
            .unwrap();
        let service = PbftVerifiedVotesService::from_runtime(storage.clone(), runtime);
        let cursor = RewardVoteCursor {
            period: 12,
            round: 2,
            step: 3,
            block_hash: H256::from(block_hash),
        };
        let prepare_request = RewardVoteResetPrepareRequest {
            requested: true,
            cursor,
        };

        let stage = service
            .prepare_reward_votes_reset_stage(prepare_request)
            .unwrap();
        assert_eq!(stage.stage, 4);
        assert!(stage.has_reward_votes_reset);
        assert!(!stage.reward_votes_bundle_rlp.is_empty());
        let mismatched = RewardVoteResetPrepareRequest {
            cursor: RewardVoteCursor {
                block_hash: H256::from([0x62; 32]),
                ..cursor
            },
            ..prepare_request
        };
        assert!(
            service
                .prepare_reward_votes_reset_stage(mismatched)
                .unwrap_err()
                .to_string()
                .contains("PBFT_REWARD_VOTES_RESET_CERT_IDENTITY_MISMATCH")
        );

        let applied = service
            .apply_reward_votes_reset(RewardVoteResetApplyRequest {
                period: cursor.period,
                round: cursor.round,
                step: cursor.step,
                block_hash: cursor.block_hash,
                sync: false,
            })
            .unwrap();
        assert_eq!(applied.status, PbftFinalizedPeriodApplyStatus::Applied);
        assert_ne!(applied.reward_votes_reset_generation, 0);
        assert!(
            storage
                .pbft()
                .extra_reward_vote_hashes()
                .unwrap()
                .is_empty()
        );

        let snapshot = service.current_reward_vote_snapshot().unwrap();
        assert!(snapshot.cursor.found);
        assert_eq!(snapshot.cursor.period, cursor.period);
        assert_eq!(snapshot.cursor.round, cursor.round);
        assert_eq!(snapshot.cursor.step, cursor.step);
        assert_eq!(snapshot.cursor.block_hash, cursor.block_hash);
        assert_eq!(snapshot.records.len(), 2);
        assert_eq!(
            service
                .commit_reward_vote_cursor(RewardVoteCursorCommitRequest {
                    cursor,
                    reset_generation: applied.reward_votes_reset_generation,
                })
                .unwrap()
                .status,
            RewardVoteCursorCommitStatus::AlreadyCurrent
        );
        let repeated = service
            .apply_reward_votes_reset(RewardVoteResetApplyRequest {
                period: cursor.period,
                round: cursor.round,
                step: cursor.step,
                block_hash: cursor.block_hash,
                sync: false,
            })
            .unwrap();
        assert_eq!(
            repeated.status,
            PbftFinalizedPeriodApplyStatus::AlreadyAppliedSameValues
        );
        assert!(repeated.reward_votes_reset_generation > applied.reward_votes_reset_generation);

        {
            let mut runtime = service.lock().unwrap();
            for (period, round, alternative_hash, secret) in [
                (11, 1, [0x51; 32], NODE_SECRET),
                (12, 1, [0x52; 32], NODE_SECRET_TWO),
            ] {
                let canonical = vote_rlp_for(alternative_hash, period, round, 3, secret);
                let inspection = validation(&canonical);
                let weighted =
                    build_weighted_pbft_vote_payload(&canonical, inspection.calculated_weight)
                        .unwrap();
                runtime
                    .verified_votes_mut()
                    .add_verified_vote(
                        VerifiedVote::new(
                            inspection.vote_hash,
                            inspection.block_hash,
                            inspection.recovered_voter,
                            inspection.period,
                            inspection.round,
                            inspection.step,
                            inspection.vote_type,
                            inspection.calculated_weight,
                        )
                        .unwrap(),
                        None,
                    )
                    .unwrap();
                runtime.payloads.insert(
                    weighted.hash,
                    PbftVoteRuntimePayload {
                        slashing: build_slashing_pbft_vote_payload(&canonical).unwrap(),
                        weighted,
                    },
                );
                assert!(
                    runtime
                        .verified_votes_mut()
                        .insert_two_t_plus_one_voted_block(
                            period,
                            round,
                            TwoTPlusOneVotedBlockType::CertVotedBlock,
                            H256::from(alternative_hash),
                            3,
                        )
                        .round_found
                );
            }
        }
        let durable_before_rejections = storage.pbft().finalized_reward_vote_cursor().unwrap();
        let bundle_before_rejections = storage
            .get_raw(Column::LatestRoundTwoTPlusOneVotes, &[1])
            .unwrap();
        let generation_before_rejections = storage.extra_reward_votes_reset_generation();
        for rejected_cursor in [
            RewardVoteCursor {
                period: 11,
                round: 1,
                step: 3,
                block_hash: H256::from([0x51; 32]),
            },
            RewardVoteCursor {
                period: 12,
                round: 1,
                step: 3,
                block_hash: H256::from([0x52; 32]),
            },
        ] {
            assert!(
                service
                    .apply_reward_votes_reset(RewardVoteResetApplyRequest {
                        period: rejected_cursor.period,
                        round: rejected_cursor.round,
                        step: rejected_cursor.step,
                        block_hash: rejected_cursor.block_hash,
                        sync: false,
                    })
                    .unwrap_err()
                    .to_string()
                    .contains("PBFT_REWARD_CURSOR_NOT_MONOTONIC")
            );
            assert_eq!(
                storage.extra_reward_votes_reset_generation(),
                generation_before_rejections
            );
            assert_eq!(
                storage.pbft().finalized_reward_vote_cursor().unwrap(),
                durable_before_rejections
            );
            assert_eq!(
                storage
                    .get_raw(Column::LatestRoundTwoTPlusOneVotes, &[1],)
                    .unwrap(),
                bundle_before_rejections
            );
            assert_eq!(
                service.reward_vote_cursor_snapshot().unwrap(),
                snapshot.cursor
            );
        }

        drop(service);
        let restarted = PbftVerifiedVotesService::restore(storage.clone()).unwrap();
        assert_eq!(
            restarted.reward_vote_cursor_snapshot().unwrap(),
            snapshot.cursor
        );
        assert_eq!(
            restarted.current_reward_vote_snapshot().unwrap().records,
            snapshot.records
        );

        drop(restarted);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn verified_votes_service_restores_from_storage_and_survives_runtime_clones() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("rustaxa_verified_votes_service_restore_{nonce}"));
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let canonical = vote_rlp([20; 32], 3);
        let weighted = build_weighted_pbft_vote_payload(&canonical, 7).unwrap();
        storage
            .pbft()
            .write_own_verified_vote(weighted.hash, &weighted.vote_rlp)
            .unwrap();
        storage
            .pbft()
            .write_extra_reward_vote(weighted.hash, &weighted.vote_rlp)
            .unwrap();

        let service = PbftVerifiedVotesService::restore(storage.clone()).unwrap();
        {
            let runtime = service.lock().unwrap();
            assert_eq!(runtime.verified_votes().size(), 1);
            assert_eq!(runtime.current_reward_vote_payloads().unwrap(), Vec::new());
        }

        let clone = service.clone();
        drop(service);
        let runtime = clone.lock().unwrap();
        assert_eq!(
            runtime.weighted_payload(weighted.hash),
            Some(&PbftVotePayloadRecord {
                hash: weighted.hash,
                vote_rlp: weighted.vote_rlp.clone(),
            })
        );
        drop(runtime);
        drop(storage);
        let storage_ref = &clone;
        assert_eq!(
            storage_ref
                .storage()
                .period()
                .by_pbft_hash(H256::zero())
                .unwrap(),
            None
        );

        drop(clone);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn verified_votes_service_lock_shares_state_across_clones() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("rustaxa_verified_votes_service_lock_{nonce}"));
        let service = PbftVerifiedVotesService::from_runtime(
            Arc::new(Storage::new(Config::new(path.clone())).unwrap()),
            PbftVoteAdmissionRuntime::new(),
        );
        let service_a = service.clone();
        let service_b = service.clone();

        let first_vote = vote_rlp([30; 32], 3);
        let first_validation = validation(&first_vote);
        {
            let mut runtime = service_a.lock().unwrap();
            runtime
                .admit_validated_vote(&first_vote, &first_validation, flags(), context(Some(80)))
                .unwrap();
            assert_eq!(runtime.verified_votes().size(), 1);
            runtime.reward_vote_cursor = Some(RewardVoteCursor {
                period: first_validation.period,
                round: first_validation.round,
                step: first_validation.step,
                block_hash: first_validation.block_hash,
            });
        }

        {
            let runtime = service_b.lock().unwrap();
            assert_eq!(runtime.verified_votes().size(), 1);
            assert!(runtime.replay_contains(first_validation.vote_hash));
            assert_eq!(
                runtime.reward_vote_cursor,
                Some(RewardVoteCursor {
                    period: first_validation.period,
                    round: first_validation.round,
                    step: first_validation.step,
                    block_hash: first_validation.block_hash,
                })
            );
        }

        let runtime = service.lock().unwrap();
        assert_eq!(runtime.verified_votes().size(), 1);
        assert!(runtime.replay_contains(first_validation.vote_hash));
        drop(runtime);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn verified_votes_service_rejects_malformed_durable_rows_before_publication() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("rustaxa_verified_votes_service_malformed_{nonce}"));
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        storage
            .pbft()
            .write_own_verified_vote(H256::from_low_u64_be(1), &[0x01])
            .unwrap();

        let _error = PbftVerifiedVotesService::restore(storage.clone())
            .err()
            .expect("malformed durable vote must prevent service publication");

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn verified_votes_service_owns_access_queries_bundles_and_cleanup() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustaxa_verified_access_{nonce}"));
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let block_hash = [0x71; 32];
        let first = vote_rlp_from_secret(block_hash, 3, NODE_SECRET);
        let second = vote_rlp_from_secret(block_hash, 3, NODE_SECRET_TWO);
        let first_validation = validation(&first);
        let second_validation = validation(&second);
        let mut runtime = PbftVoteAdmissionRuntime::new();
        runtime
            .admit_validated_vote(&first, &first_validation, flags(), context(Some(80)))
            .unwrap();
        runtime
            .admit_validated_vote(&second, &second_validation, flags(), context(Some(80)))
            .unwrap();
        let service = PbftVerifiedVotesService::from_runtime(storage.clone(), runtime);

        assert_eq!(service.verified_votes_size().unwrap(), 2);
        assert!(
            service
                .verified_votes_replay_contains(first_validation.vote_hash)
                .unwrap()
        );
        let extra_hash = H256::from([0x72; 32]);
        assert!(service.verified_votes_replay_insert(extra_hash).unwrap());
        assert!(!service.verified_votes_replay_insert(extra_hash).unwrap());

        let mapping = service
            .verified_votes_get_two_t_plus_one_voted_block(
                12,
                2,
                TwoTPlusOneVotedBlockType::CertVotedBlock,
            )
            .unwrap()
            .unwrap();
        assert_eq!(mapping.block_hash, H256::from(block_hash));
        assert_eq!(mapping.step, 3);
        let payloads = service
            .verified_votes_get_two_t_plus_one_voted_block_payloads(
                12,
                2,
                TwoTPlusOneVotedBlockType::CertVotedBlock,
            )
            .unwrap()
            .unwrap();
        assert_eq!(payloads.votes.len(), 2);
        let requested_hashes = payloads
            .votes
            .iter()
            .map(|vote| vote.hash)
            .collect::<Vec<_>>();
        let built = service
            .verified_votes_build_optimized_votes_bundle_egress(
                PbftOptimizedVoteBundleBuildRequest {
                    kind: TwoTPlusOneVotedBlockType::CertVotedBlock,
                    block_hash: mapping.block_hash,
                    period: 12,
                    round: 2,
                    step: mapping.step,
                    vote_hashes: requested_hashes.clone(),
                },
            )
            .unwrap();
        assert_eq!(built.status, VerifiedVoteOptimizedBundleStatus::Ready);
        assert_eq!(built.vote_hashes, requested_hashes);
        assert!(!built.votes_bundle_rlp.is_empty());
        let mut reversed_hashes = requested_hashes.clone();
        reversed_hashes.reverse();
        assert_eq!(
            service
                .verified_votes_build_optimized_votes_bundle_egress(
                    PbftOptimizedVoteBundleBuildRequest {
                        kind: TwoTPlusOneVotedBlockType::CertVotedBlock,
                        block_hash: mapping.block_hash,
                        period: 12,
                        round: 2,
                        step: mapping.step,
                        vote_hashes: reversed_hashes,
                    },
                )
                .unwrap()
                .status,
            VerifiedVoteOptimizedBundleStatus::OrderMismatch
        );
        assert_eq!(
            service
                .verified_votes_build_optimized_votes_bundle_egress(
                    PbftOptimizedVoteBundleBuildRequest {
                        kind: TwoTPlusOneVotedBlockType::CertVotedBlock,
                        block_hash: H256::zero(),
                        period: 12,
                        round: 2,
                        step: mapping.step,
                        vote_hashes: requested_hashes.clone(),
                    },
                )
                .unwrap()
                .status,
            VerifiedVoteOptimizedBundleStatus::MappingMismatch
        );
        let next_plan = service
            .verified_votes_plan_next_votes_bundle_egress(12, 2)
            .unwrap();
        assert_eq!(
            next_plan.next_votes.status,
            VerifiedVoteOptimizedBundleStatus::NotFound
        );
        assert_eq!(next_plan.next_votes.period, 12);
        assert_eq!(next_plan.next_votes.round, 2);
        assert!(next_plan.next_votes.plan.is_none());
        assert_eq!(next_plan.next_null_votes.period, 12);
        assert_eq!(next_plan.next_null_votes.round, 2);

        let snapshot = service.verified_votes_state_snapshot().unwrap();
        assert_eq!(snapshot.votes.len(), 2);
        let step = service
            .verified_votes_step_payloads(12, 2, 3)
            .unwrap()
            .unwrap();
        assert_eq!(step.len(), 1);
        assert_eq!(step[0].votes.len(), 2);
        assert!(
            service
                .verified_votes_step_payloads(12, 2, 99)
                .unwrap()
                .is_none()
        );

        service.verified_votes_cleanup_votes_by_period(13).unwrap();
        assert_eq!(service.verified_votes_size().unwrap(), 0);
        assert!(
            service
                .verified_votes_state_snapshot()
                .unwrap()
                .votes
                .is_empty()
        );

        drop(service);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn verified_votes_direct_persistence_waits_for_runtime_serialization() {
        use std::sync::mpsc;
        use std::time::Duration;

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustaxa_verified_persist_lock_{nonce}"));
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let service = PbftVerifiedVotesService::from_runtime(
            storage.clone(),
            PbftVoteAdmissionRuntime::new(),
        );
        let guard = service.lock().unwrap();
        let worker = service.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = worker
                .verified_votes_persist_pbft_vote_progress(
                    PbftVerifiedVoteProgressPersistenceWrite::default(),
                )
                .unwrap();
            done_tx.send(result).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(guard);
        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(2)).unwrap().status,
            PbftVotePersistenceStatus::Applied
        );
        thread.join().unwrap();

        drop(service);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn verified_votes_service_validates_saves_reads_and_clears_own_votes() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustaxa_verified_own_votes_{nonce}"));
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let service = PbftVerifiedVotesService::from_runtime(
            storage.clone(),
            PbftVoteAdmissionRuntime::new(),
        );
        let first_canonical = vote_rlp_from_secret([0x81; 32], 3, NODE_SECRET);
        let second_canonical = vote_rlp_from_secret([0x82; 32], 3, NODE_SECRET_TWO);
        for (canonical, weight) in [(&second_canonical, 5), (&first_canonical, 4)] {
            assert_eq!(
                service
                    .verified_votes_save_own_verified_vote(canonical, weight)
                    .unwrap()
                    .status,
                PbftVotePersistenceStatus::Applied
            );
        }
        let records = service.verified_votes_own_vote_records().unwrap();
        assert_eq!(records.len(), 2);
        assert!(records.windows(2).all(|pair| pair[0].hash < pair[1].hash));

        assert!(
            service
                .verified_votes_save_own_verified_vote(&first_canonical, 0)
                .unwrap_err()
                .to_string()
                .contains("non-zero weight")
        );
        assert_eq!(
            service
                .verified_votes_clear_own_verified_votes()
                .unwrap()
                .status,
            PbftVotePersistenceStatus::Applied
        );
        assert!(
            service
                .verified_votes_own_vote_records()
                .unwrap()
                .is_empty()
        );

        drop(service);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }
}
