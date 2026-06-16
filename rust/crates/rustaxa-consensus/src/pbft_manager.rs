//! Deterministic PBFT manager runtime planning and storage-backed startup restore.
//!
//! This module owns Rust-side PBFT manager orchestration: storage-backed startup
//! restoration for the long-lived runtime plus the ordered control-flow script
//! for one daemon tick. Tick planning is intentionally side-effect-free. C++
//! supplies already-collected live facts, executes each requested action against
//! the existing manager shell, then reports the result before Rust advances the
//! cursor. Eligible-wallet state is reported after the pre-state cert/round
//! checks so the runtime preserves the legacy branch order.
//!
//! Inputs are a compact `PbftManagerRuntimeTickFact`: current PBFT state,
//! period/round/step telemetry, network availability, sync status, and whether
//! any local wallet is eligible for the current period. Outputs are stable
//! action/status codes and a cursor-managed session.
//!
//! Invariants:
//! - Rust decides the order of manager actions for the tick.
//! - C++ remains the temporary owner of live objects, network dispatch, sleeps,
//!   and non-migrated state mutation in this slice.
//! - Storage-backed startup reads and step normalization use
//!   `rustaxa_storage::Storage` directly inside Rust.
//! - Early-progress actions such as cert-block push complete the session with
//!   `restart_loop = true`, matching the old `continue` path.
//! - Round-advance candidates are reported as facts. Rust validates them and
//!   emits an explicit `ResetConsensus` effect with the target round.
//! - The active-state vs ineligible-sleep branch is selected from the
//!   `has_eligible_wallet` report supplied after `TryAdvanceRound`.
//! - Branches after `run_certify` and `run_second_finish` are selected only from
//!   explicit report flags returned by the C++ executor.

use anyhow::{Context, Result, anyhow};
use ethereum_types::H256;
use rlp::RlpStream;
use rustaxa_storage::{Storage, StorageWriteBatch};
use rustaxa_types::codec::rlp::dag::FinalizedDagBlockBundleRlp;
use std::collections::{BTreeMap, VecDeque};
use tiny_keccak::{Hasher, Keccak};

const PBFT_MGR_FIELD_ROUND: u8 = 0;
const PBFT_MGR_FIELD_STEP: u8 = 1;
const PBFT_MGR_FIELD_LAMBDA: u8 = 2;
const PBFT_MGR_STATUS_EXECUTED_BLOCK: u8 = 0;
const PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE: u8 = 2;
const PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH: u8 = 3;

/// Stable PBFT manager state codes used by the CXX bridge.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerRuntimeStateCode {
    /// Value-proposal state.
    ValueProposal,
    /// Filtering / identify-leader state.
    Filter,
    /// Certifying state.
    Certify,
    /// First finish state.
    Finish,
    /// Second finish / polling state.
    FinishPolling,
    /// Unknown bridge state.
    Unknown,
}

/// Live-object availability status for one proposed PBFT leader candidate.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerLeaderCandidateStatus {
    /// The candidate block resolved, passed current validation, and can be selected.
    Ready,
    /// The proposal vote pointed at the null PBFT block hash and must be ignored.
    NullVoteBlockHash,
    /// The candidate PBFT block is already present in the local PBFT chain.
    BlockInChain,
    /// C++ could not resolve or validate the proposed block for the vote.
    BlockMissingOrInvalid,
    /// The vote did not carry a positive proposer weight.
    InvalidVoteWeight,
    /// Unknown bridge status.
    Unknown,
}

/// Validation result for a proposed PBFT block candidate.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerLeaderBlockValidationStatus {
    /// The block was already marked valid in the proposed-block sidecar.
    AlreadyValid,
    /// C++ live validation accepted the block.
    Validated,
    /// C++ live validation rejected the block.
    Rejected,
    /// Unknown bridge status.
    Unknown,
}

/// Live validation status for one proposed-block admission attempt.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerCandidateAdmissionValidationStatus {
    /// Rust has not requested live block validation yet.
    NotChecked,
    /// C++ live validation accepted the proposed block.
    Valid,
    /// C++ live validation rejected the proposed block.
    Invalid,
    /// Unknown bridge status.
    Unknown,
}

impl PbftManagerCandidateAdmissionValidationStatus {
    /// Stable bridge code for proposed-block admission validation status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::NotChecked => 0,
            Self::Valid => 1,
            Self::Invalid => 2,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge status code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::NotChecked,
            1 => Self::Valid,
            2 => Self::Invalid,
            _ => Self::Unknown,
        }
    }
}

/// Runtime action for Rust-owned proposed-block admission.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerCandidateAdmissionAction {
    /// C++ must lookup the live proposed block sidecar before retrying.
    RequestLookup,
    /// C++ must validate the found block and report the result.
    RequestValidation,
    /// The block is accepted for use by the caller.
    Accept,
    /// The block is rejected by supplied facts.
    Reject,
    /// Supplied bridge facts violate the admission contract.
    ContractError,
}

impl PbftManagerCandidateAdmissionAction {
    /// Stable bridge code for proposed-block admission action.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::RequestLookup => 0,
            Self::RequestValidation => 1,
            Self::Accept => 2,
            Self::Reject => 3,
            Self::ContractError => 255,
        }
    }
}

/// Final proposed-block admission status selected by Rust.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerCandidateAdmissionStatus {
    /// The proposed-block sidecar lookup is still needed.
    LookupRequired,
    /// Live block validation is still needed.
    ValidationRequired,
    /// The block was already marked valid and is accepted.
    AcceptedAlreadyValid,
    /// The block was newly validated and is accepted.
    AcceptedNewlyValidated,
    /// The proposed-block sidecar did not contain the requested block.
    BlockMissing,
    /// Live block validation rejected the candidate.
    ValidationRejected,
    /// Supplied bridge facts violate the admission contract.
    InvalidBridgeFacts,
}

impl PbftManagerCandidateAdmissionStatus {
    /// Stable bridge code for proposed-block admission status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::LookupRequired => 0,
            Self::ValidationRequired => 1,
            Self::AcceptedAlreadyValid => 2,
            Self::AcceptedNewlyValidated => 3,
            Self::BlockMissing => 4,
            Self::ValidationRejected => 5,
            Self::InvalidBridgeFacts => 255,
        }
    }
}

impl PbftManagerLeaderBlockValidationStatus {
    /// Stable bridge code for proposed-block validation status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::AlreadyValid => 0,
            Self::Validated => 1,
            Self::Rejected => 2,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge status code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::AlreadyValid,
            1 => Self::Validated,
            2 => Self::Rejected,
            _ => Self::Unknown,
        }
    }
}

impl PbftManagerLeaderCandidateStatus {
    /// Stable bridge code for the candidate status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::NullVoteBlockHash => 1,
            Self::BlockInChain => 2,
            Self::BlockMissingOrInvalid => 3,
            Self::InvalidVoteWeight => 4,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge status code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Ready,
            1 => Self::NullVoteBlockHash,
            2 => Self::BlockInChain,
            3 => Self::BlockMissingOrInvalid,
            4 => Self::InvalidVoteWeight,
            _ => Self::Unknown,
        }
    }
}

/// Live facts for one proposal vote before Rust derives candidate status.
///
/// Inputs:
/// - `vote_hash`, `block_hash`, `period`, `credential`, and
///   `voter_public_key` identify and rank the proposal vote.
/// - `weight_found` and `weight` describe the validated proposer weight.
/// - `block_in_chain`, `proposed_block_found`, and `block_validation_status`
///   summarize C++ live sidecar/PBFT-chain/DAG validation without deciding
///   candidate eligibility in C++.
/// - `pivot_hash` is the proposed block pivot hash when the block was found.
///
/// Outputs are produced by `plan_pbft_manager_leader_candidates`.
///
/// Invariants:
/// - Rust owns the legacy candidate-status derivation and ranking.
/// - C++ remains responsible for live object lookup and block validation until
///   those dependencies move into Rust.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerLeaderCandidateInputFact {
    /// Signed proposal vote hash.
    pub vote_hash: H256,
    /// Proposed PBFT block hash from the proposal vote.
    pub block_hash: H256,
    /// Vote period used for live block lookup.
    pub period: u64,
    /// Proposal vote VRF output.
    pub credential: [u8; 64],
    /// Recovered voter public key.
    pub voter_public_key: [u8; 64],
    /// True when proposer weight was present on the live vote.
    pub weight_found: bool,
    /// Validated proposer vote weight.
    pub weight: u64,
    /// True when the proposed block is already in the PBFT chain.
    pub block_in_chain: bool,
    /// True when the proposed-block sidecar resolved the block.
    pub proposed_block_found: bool,
    /// Proposed-block validation status.
    pub block_validation_status: PbftManagerLeaderBlockValidationStatus,
    /// Pivot DAG hash for a found proposed block.
    pub pivot_hash: H256,
}

/// C++-originated facts for one proposed PBFT block admission attempt.
///
/// Inputs:
/// - `period` and `block_hash` identify the candidate the caller wants to use.
/// - `lookup_performed`, `proposed_block_found`, and
///   `proposed_block_already_valid` report the proposed-block sidecar lookup.
/// - `validation_status` reports the live validation result only after Rust
///   asks for validation.
///
/// Outputs are produced by `plan_pbft_manager_candidate_admission`.
///
/// Invariants and edge behavior:
/// - Rust owns the admission state machine and mark-valid decision.
/// - C++ owns the live sidecar lookup, block validation checks, and sidecar
///   mutation requested by the final plan.
/// - Missing blocks and failed validation are explicit rejections; malformed
///   fact order returns a contract error.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerCandidateAdmissionFact {
    /// Candidate PBFT period.
    pub period: u64,
    /// Candidate PBFT block hash.
    pub block_hash: H256,
    /// True once C++ has looked up the proposed-block sidecar.
    pub lookup_performed: bool,
    /// True when the proposed-block sidecar resolved the candidate block.
    pub proposed_block_found: bool,
    /// True when the resolved proposed block was already marked valid.
    pub proposed_block_already_valid: bool,
    /// Live validation result supplied after Rust requests validation.
    pub validation_status: PbftManagerCandidateAdmissionValidationStatus,
}

/// Side-effect-free proposed-block admission plan for C++ execution.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerCandidateAdmissionPlan {
    /// Runtime action C++ must take.
    pub action: PbftManagerCandidateAdmissionAction,
    /// Current admission status.
    pub status: PbftManagerCandidateAdmissionStatus,
    /// True when C++ must mark the proposed block valid before returning it.
    pub mark_valid: bool,
    /// Stable diagnostic code for bridge/log consumers.
    pub error_code: &'static str,
}

/// Rust-owned outcome for PBFT leader candidate selection.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerLeaderSelectionStatus {
    /// A leader block/vote pair was selected.
    Selected,
    /// No proposal vote facts were supplied.
    Empty,
    /// Candidate facts were present, but none were selectable.
    NoEligibleCandidate,
    /// One or more candidate facts were malformed.
    InvalidFact,
}

impl PbftManagerLeaderSelectionStatus {
    /// Stable bridge code for the selection status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Selected => 0,
            Self::Empty => 1,
            Self::NoEligibleCandidate => 2,
            Self::InvalidFact => 3,
        }
    }
}

/// Candidate facts for deterministic PBFT leader selection.
///
/// Inputs:
/// - `vote_hash` and `block_hash` identify the live C++ objects to materialize
///   after Rust selection.
/// - `credential` is the 64-byte VRF output from the proposal vote.
/// - `voter_public_key` is the 64-byte secp256k1 public key recovered from the
///   vote signature.
/// - `weight` is the already-validated proposer vote weight.
/// - `status` and `pivot_hash` summarize C++ live-object resolution and
///   candidate validation. Rust uses these facts only after applying legacy
///   proposal ranking.
///
/// Outputs are produced by `plan_pbft_manager_leader_selection`.
///
/// Invariants:
/// - Candidate ordering is computed from the legacy minimum of
///   `sha3(rlp([credential, voter_public_key, i]))` for `i = 1..=weight`.
/// - Duplicate rank hashes retain the last input candidate, matching legacy
///   `std::map<h256, vote>` assignment behavior.
/// - Null-anchor candidates are eligible only as a fallback when no non-null
///   candidate wins.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerLeaderCandidateFact {
    /// Signed proposal vote hash.
    pub vote_hash: H256,
    /// Proposed PBFT block hash.
    pub block_hash: H256,
    /// Vote period used for live block lookup.
    pub period: u64,
    /// Proposal vote VRF output.
    pub credential: [u8; 64],
    /// Recovered voter public key.
    pub voter_public_key: [u8; 64],
    /// Validated proposal vote weight.
    pub weight: u64,
    /// Candidate live-object/validation status.
    pub status: PbftManagerLeaderCandidateStatus,
    /// Pivot DAG hash for a ready candidate block.
    pub pivot_hash: H256,
}

/// Side-effect-free PBFT leader selection plan.
///
/// The selected hashes identify the C++ live vote/block pair to return from the
/// shim. Empty and rejected plans return zero hashes and `selected = false`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerLeaderSelectionPlan {
    /// Selection status.
    pub status: PbftManagerLeaderSelectionStatus,
    /// True when `selected_vote_hash` and `selected_block_hash` are meaningful.
    pub selected: bool,
    /// Selected proposal vote hash.
    pub selected_vote_hash: H256,
    /// Selected PBFT block hash.
    pub selected_block_hash: H256,
    /// Selected vote period.
    pub selected_period: u64,
    /// True when the selected block is the null-anchor fallback.
    pub selected_from_null_anchor: bool,
    /// Stable diagnostic code for bridge/log consumers.
    pub error_code: &'static str,
}

/// Proposed-block command emitted by grouped PBFT candidate planning.
///
/// C++ applies this command to mark a proposed PBFT block valid only after Rust
/// has accepted the corresponding validation report.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerLeaderValidBlockCommand {
    /// PBFT period for the proposed block.
    pub period: u64,
    /// Proposed PBFT block hash to mark valid.
    pub block_hash: H256,
}

/// Grouped PBFT leader-candidate plan.
///
/// The selection fields mirror `PbftManagerLeaderSelectionPlan`. The
/// `valid_blocks` commands are emitted for unmarked candidate blocks whose live
/// validation was reported as accepted, keeping proposed-block status mutation
/// under the Rust-planned route.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerLeaderCandidatePlan {
    /// Selection status.
    pub status: PbftManagerLeaderSelectionStatus,
    /// True when `selected_vote_hash` and `selected_block_hash` are meaningful.
    pub selected: bool,
    /// Selected proposal vote hash.
    pub selected_vote_hash: H256,
    /// Selected PBFT block hash.
    pub selected_block_hash: H256,
    /// Selected vote period.
    pub selected_period: u64,
    /// True when the selected block is the null-anchor fallback.
    pub selected_from_null_anchor: bool,
    /// Proposed blocks that C++ should mark valid.
    pub valid_blocks: Vec<PbftManagerLeaderValidBlockCommand>,
    /// Stable diagnostic code for bridge/log consumers.
    pub error_code: &'static str,
}

/// Tri-state fact status for Rust-owned PBFT block validation orchestration.
///
/// C++ reports each live-object check with this status after Rust asks for the
/// next check. `Missing` is distinct from `Invalid` for FinalChain lag and DAG
/// order availability, where the caller may choose to retry or delay instead of
/// treating the peer/block as malicious.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerBlockValidationFactStatus {
    /// The fact has not been supplied yet.
    NotChecked,
    /// The live check accepted the fact.
    Valid,
    /// The live check rejected the fact.
    Invalid,
    /// The live check could not resolve required data.
    Missing,
    /// The check is not required for this block/context.
    NotRequired,
    /// Unknown bridge status.
    Unknown,
}

impl PbftManagerBlockValidationFactStatus {
    /// Stable bridge code for a PBFT block-validation fact status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::NotChecked => 0,
            Self::Valid => 1,
            Self::Invalid => 2,
            Self::Missing => 3,
            Self::NotRequired => 4,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge status code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::NotChecked,
            1 => Self::Valid,
            2 => Self::Invalid,
            3 => Self::Missing,
            4 => Self::NotRequired,
            _ => Self::Unknown,
        }
    }
}

/// Next live check requested by Rust PBFT block validation planning.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerBlockValidationNextCheck {
    /// No further live check is needed.
    None,
    /// Validate the block's previous PBFT hash against the current PBFT chain.
    CheckPbftChain,
    /// Validate the PBFT block FinalChain/state-root hash.
    ValidateFinalChainHash,
    /// Check reward-vote availability/validity for the candidate block.
    CheckRewardVotes,
    /// Validate PBFT block extra-data shape for the active hardfork.
    ValidateExtraData,
    /// Compare the embedded pillar block hash against the local pillar block.
    ValidatePillarBlock,
    /// Resolve and verify DAG order for the candidate pivot.
    CheckDagOrder,
    /// Check DAG block weight after Rust requested and C++ cached the order.
    CheckDagWeight,
    /// Unknown bridge status.
    Unknown,
}

impl PbftManagerBlockValidationNextCheck {
    /// Stable bridge code for the next requested check.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::None => 255,
            Self::CheckPbftChain => 0,
            Self::ValidateFinalChainHash => 1,
            Self::CheckRewardVotes => 2,
            Self::ValidateExtraData => 3,
            Self::ValidatePillarBlock => 4,
            Self::CheckDagOrder => 5,
            Self::CheckDagWeight => 6,
            Self::Unknown => 254,
        }
    }
}

/// PBFT block-validation runtime action selected by Rust.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerBlockValidationAction {
    /// C++ must execute `next_check` and call the planner again with the result.
    RunCheck,
    /// The block is accepted by all required checks.
    Accept,
    /// The block is rejected by a supplied fact.
    Reject,
    /// The FinalChain/state-root fact is missing and the caller may wait/retry.
    WaitForFinalization,
    /// Supplied bridge facts violate the validation contract.
    ContractError,
}

impl PbftManagerBlockValidationAction {
    /// Stable bridge code for a PBFT block-validation action.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::RunCheck => 0,
            Self::Accept => 1,
            Self::Reject => 2,
            Self::WaitForFinalization => 3,
            Self::ContractError => 255,
        }
    }
}

/// Final PBFT block-validation status selected by Rust.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerBlockValidationStatus {
    /// Validation is still waiting for C++ to run a requested check.
    Pending,
    /// All required checks accepted the PBFT block.
    Accepted,
    /// Previous PBFT hash/chain validation failed.
    PbftChainInvalid,
    /// FinalChain/state-root validation is behind execution.
    FinalChainHashMissing,
    /// FinalChain/state-root validation rejected the block.
    FinalChainHashInvalid,
    /// Reward votes rejected the block.
    RewardVotesInvalid,
    /// Extra-data shape rejected the block.
    ExtraDataInvalid,
    /// Embedded/local pillar block facts rejected the block.
    PillarBlockInvalid,
    /// DAG order could not be resolved.
    DagOrderMissing,
    /// DAG order hash rejected the block.
    DagOrderInvalid,
    /// DAG block weight rejected the block.
    DagWeightInvalid,
    /// Supplied bridge facts violate the validation contract.
    InvalidBridgeFacts,
}

impl PbftManagerBlockValidationStatus {
    /// Stable bridge code for a PBFT block-validation status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Pending => 0,
            Self::Accepted => 1,
            Self::PbftChainInvalid => 2,
            Self::FinalChainHashMissing => 3,
            Self::FinalChainHashInvalid => 4,
            Self::RewardVotesInvalid => 5,
            Self::ExtraDataInvalid => 6,
            Self::PillarBlockInvalid => 7,
            Self::DagOrderMissing => 8,
            Self::DagOrderInvalid => 9,
            Self::DagWeightInvalid => 10,
            Self::InvalidBridgeFacts => 255,
        }
    }
}

/// Compact fact bundle for Rust-owned PBFT block validation orchestration.
///
/// Inputs:
/// - Block identity fields let C++ correlate diagnostics and cached DAG state.
/// - `*_status` fields report the result of live checks only after Rust asks for
///   the corresponding `next_check`.
/// - `pivot_is_null`, `dag_order_cached`, `dag_order_required`,
///   `pillar_block_required`, and `dag_weight_check_required` encode
///   deterministic branch conditions that C++ can derive from existing sidecars
///   without deciding final acceptance.
///
/// Outputs are produced by `plan_pbft_manager_block_validation`.
///
/// Invariants and edge behavior:
/// - Rust owns the ordering of all validation checks.
/// - C++ owns live PBFT chain, FinalChain, reward-vote, pillar, and DAG queries.
/// - Missing FinalChain hash facts return `WaitForFinalization`; proposal paths
///   may treat that as rejection, while sync paths can wait and retry.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerBlockValidationFact {
    /// Candidate PBFT block hash.
    pub block_hash: H256,
    /// Candidate PBFT period.
    pub period: u64,
    /// Candidate pivot DAG block hash.
    pub pivot_hash: H256,
    /// True when the pivot hash is the null DAG anchor.
    pub pivot_is_null: bool,
    /// True when the C++ DAG-order sidecar already has cached order for pivot.
    pub dag_order_cached: bool,
    /// True when this validation context requires DAG order/hash validation.
    pub dag_order_required: bool,
    /// True when hardfork rules require local pillar-block hash comparison.
    pub pillar_block_required: bool,
    /// True when the resolved DAG order must pass the weight check.
    pub dag_weight_check_required: bool,
    /// PBFT-chain previous-hash validation status.
    pub pbft_chain_status: PbftManagerBlockValidationFactStatus,
    /// FinalChain/state-root validation status.
    pub final_chain_hash_status: PbftManagerBlockValidationFactStatus,
    /// Reward-vote validation status.
    pub reward_votes_status: PbftManagerBlockValidationFactStatus,
    /// Extra-data validation status.
    pub extra_data_status: PbftManagerBlockValidationFactStatus,
    /// Pillar block validation status.
    pub pillar_block_status: PbftManagerBlockValidationFactStatus,
    /// DAG order lookup/hash validation status.
    pub dag_order_status: PbftManagerBlockValidationFactStatus,
    /// DAG weight validation status.
    pub dag_weight_status: PbftManagerBlockValidationFactStatus,
}

/// Side-effect-free PBFT block-validation plan for C++ execution.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerBlockValidationPlan {
    /// Runtime action C++ must take.
    pub action: PbftManagerBlockValidationAction,
    /// Current validation status.
    pub status: PbftManagerBlockValidationStatus,
    /// Next live-object check requested when `action == RunCheck`.
    pub next_check: PbftManagerBlockValidationNextCheck,
    /// Stable diagnostic code for bridge/log consumers.
    pub error_code: &'static str,
}

/// Computes the legacy PBFT proposer ranking hash for one vote index.
///
/// Inputs are the proposal vote VRF output, recovered voter public key, and
/// one-based vote-weight index. The output matches C++ `getVoterIndexHash`:
/// Keccak256 over RLP list `[credential, voter_public_key, index]`.
#[must_use]
pub fn pbft_manager_voter_index_hash(
    credential: [u8; 64],
    voter_public_key: [u8; 64],
    index: u64,
) -> H256 {
    let mut stream = RlpStream::new_list(3);
    stream.append(&credential.as_slice());
    stream.append(&voter_public_key.as_slice());
    stream.append(&index);
    keccak256(&stream.out())
}

/// Computes the legacy proposal rank for a weighted PBFT proposal vote.
///
/// The rank is the lowest voter-index hash across the vote's positive weight.
/// A zero weight has no valid rank and returns `None` so callers can surface an
/// explicit invalid fact instead of silently selecting the vote.
#[must_use]
pub fn pbft_manager_proposal_rank_hash(
    credential: [u8; 64],
    voter_public_key: [u8; 64],
    weight: u64,
) -> Option<H256> {
    if weight == 0 {
        return None;
    }

    let mut lowest_hash = pbft_manager_voter_index_hash(credential, voter_public_key, 1);
    for index in 2..=weight {
        let candidate = pbft_manager_voter_index_hash(credential, voter_public_key, index);
        if lowest_hash > candidate {
            lowest_hash = candidate;
        }
    }
    Some(lowest_hash)
}

/// Plans one proposed PBFT block admission attempt.
///
/// C++ supplies live sidecar lookup and validation facts in the order Rust
/// requests them. Rust decides whether the block is missing, needs validation,
/// should be returned immediately, should be marked valid, or must be rejected.
/// The planner does not materialize or mutate proposed blocks.
#[must_use]
pub fn plan_pbft_manager_candidate_admission(
    fact: PbftManagerCandidateAdmissionFact,
) -> PbftManagerCandidateAdmissionPlan {
    if fact.block_hash == H256::zero() {
        return pbft_manager_candidate_admission_contract_error(
            "PBFT_MANAGER_CANDIDATE_ADMISSION_ZERO_BLOCK_HASH",
        );
    }
    if !fact.lookup_performed {
        if fact.proposed_block_found
            || fact.proposed_block_already_valid
            || fact.validation_status != PbftManagerCandidateAdmissionValidationStatus::NotChecked
        {
            return pbft_manager_candidate_admission_contract_error(
                "PBFT_MANAGER_CANDIDATE_ADMISSION_PRELOOKUP_FACTS",
            );
        }
        return PbftManagerCandidateAdmissionPlan {
            action: PbftManagerCandidateAdmissionAction::RequestLookup,
            status: PbftManagerCandidateAdmissionStatus::LookupRequired,
            mark_valid: false,
            error_code: "PBFT_MANAGER_CANDIDATE_ADMISSION_LOOKUP_REQUIRED",
        };
    }

    if !fact.proposed_block_found {
        if fact.proposed_block_already_valid
            || fact.validation_status != PbftManagerCandidateAdmissionValidationStatus::NotChecked
        {
            return pbft_manager_candidate_admission_contract_error(
                "PBFT_MANAGER_CANDIDATE_ADMISSION_MISSING_BLOCK_FACTS",
            );
        }
        return PbftManagerCandidateAdmissionPlan {
            action: PbftManagerCandidateAdmissionAction::Reject,
            status: PbftManagerCandidateAdmissionStatus::BlockMissing,
            mark_valid: false,
            error_code: "PBFT_MANAGER_CANDIDATE_ADMISSION_BLOCK_MISSING",
        };
    }

    if fact.proposed_block_already_valid {
        if fact.validation_status != PbftManagerCandidateAdmissionValidationStatus::NotChecked {
            return pbft_manager_candidate_admission_contract_error(
                "PBFT_MANAGER_CANDIDATE_ADMISSION_ALREADY_VALID_WITH_REPORT",
            );
        }
        return PbftManagerCandidateAdmissionPlan {
            action: PbftManagerCandidateAdmissionAction::Accept,
            status: PbftManagerCandidateAdmissionStatus::AcceptedAlreadyValid,
            mark_valid: false,
            error_code: "PBFT_MANAGER_CANDIDATE_ADMISSION_ALREADY_VALID",
        };
    }

    match fact.validation_status {
        PbftManagerCandidateAdmissionValidationStatus::NotChecked => {
            PbftManagerCandidateAdmissionPlan {
                action: PbftManagerCandidateAdmissionAction::RequestValidation,
                status: PbftManagerCandidateAdmissionStatus::ValidationRequired,
                mark_valid: false,
                error_code: "PBFT_MANAGER_CANDIDATE_ADMISSION_VALIDATION_REQUIRED",
            }
        }
        PbftManagerCandidateAdmissionValidationStatus::Valid => PbftManagerCandidateAdmissionPlan {
            action: PbftManagerCandidateAdmissionAction::Accept,
            status: PbftManagerCandidateAdmissionStatus::AcceptedNewlyValidated,
            mark_valid: true,
            error_code: "PBFT_MANAGER_CANDIDATE_ADMISSION_VALIDATED",
        },
        PbftManagerCandidateAdmissionValidationStatus::Invalid => {
            PbftManagerCandidateAdmissionPlan {
                action: PbftManagerCandidateAdmissionAction::Reject,
                status: PbftManagerCandidateAdmissionStatus::ValidationRejected,
                mark_valid: false,
                error_code: "PBFT_MANAGER_CANDIDATE_ADMISSION_VALIDATION_REJECTED",
            }
        }
        PbftManagerCandidateAdmissionValidationStatus::Unknown => {
            pbft_manager_candidate_admission_contract_error(
                "PBFT_MANAGER_CANDIDATE_ADMISSION_UNKNOWN_VALIDATION_STATUS",
            )
        }
    }
}

/// Selects the PBFT leader candidate from C++-collected proposal facts.
///
/// C++ supplies live-object and validation status facts; Rust owns the
/// deterministic rank ordering, duplicate-rank overwrite rule, in-chain/invalid
/// skipping, and null-anchor fallback. The function does not materialize or
/// mutate blocks or votes.
#[must_use]
pub fn plan_pbft_manager_leader_selection(
    candidates: Vec<PbftManagerLeaderCandidateFact>,
) -> PbftManagerLeaderSelectionPlan {
    if candidates.is_empty() {
        return pbft_manager_leader_no_selection(
            PbftManagerLeaderSelectionStatus::Empty,
            "PBFT_MANAGER_LEADER_EMPTY",
        );
    }

    let mut ranked_candidates = BTreeMap::<H256, PbftManagerLeaderCandidateFact>::new();
    for candidate in candidates {
        if candidate.status == PbftManagerLeaderCandidateStatus::Unknown {
            return pbft_manager_leader_no_selection(
                PbftManagerLeaderSelectionStatus::InvalidFact,
                "PBFT_MANAGER_LEADER_UNKNOWN_CANDIDATE_STATUS",
            );
        }
        if candidate.weight == 0 {
            ranked_candidates.insert(
                candidate.vote_hash,
                PbftManagerLeaderCandidateFact {
                    status: PbftManagerLeaderCandidateStatus::InvalidVoteWeight,
                    ..candidate
                },
            );
            continue;
        }
        let Some(rank) = pbft_manager_proposal_rank_hash(
            candidate.credential,
            candidate.voter_public_key,
            candidate.weight,
        ) else {
            return pbft_manager_leader_no_selection(
                PbftManagerLeaderSelectionStatus::InvalidFact,
                "PBFT_MANAGER_LEADER_INVALID_WEIGHT",
            );
        };
        ranked_candidates.insert(rank, candidate);
    }

    let mut null_anchor_fallback = None;
    for candidate in ranked_candidates.into_values() {
        if candidate.status != PbftManagerLeaderCandidateStatus::Ready {
            continue;
        }
        let from_null_anchor = candidate.pivot_hash == H256::zero();
        if from_null_anchor {
            if null_anchor_fallback.is_none() {
                null_anchor_fallback = Some(candidate);
            }
            continue;
        }
        return pbft_manager_leader_selected(candidate, false);
    }

    if let Some(candidate) = null_anchor_fallback {
        return pbft_manager_leader_selected(candidate, true);
    }

    pbft_manager_leader_no_selection(
        PbftManagerLeaderSelectionStatus::NoEligibleCandidate,
        "PBFT_MANAGER_LEADER_NO_ELIGIBLE_CANDIDATE",
    )
}

/// Derives PBFT proposal candidate statuses and selects the leader.
///
/// C++ supplies compact live lookup and validation facts for every proposal
/// vote. Rust derives candidate status in the legacy order, emits mark-valid
/// commands for accepted but previously unmarked blocks, and then applies the
/// Rust-owned leader ranking/null-anchor fallback rules.
#[must_use]
pub fn plan_pbft_manager_leader_candidates(
    candidates: Vec<PbftManagerLeaderCandidateInputFact>,
) -> PbftManagerLeaderCandidatePlan {
    let mut valid_blocks = Vec::new();
    let mut selection_candidates = Vec::with_capacity(candidates.len());

    for candidate in candidates {
        let mut status = PbftManagerLeaderCandidateStatus::Ready;
        let weight = if !candidate.weight_found || candidate.weight == 0 {
            status = PbftManagerLeaderCandidateStatus::InvalidVoteWeight;
            0
        } else {
            candidate.weight
        };

        if status == PbftManagerLeaderCandidateStatus::Ready {
            if candidate.block_hash == H256::zero() {
                status = PbftManagerLeaderCandidateStatus::NullVoteBlockHash;
            } else if candidate.block_in_chain {
                status = PbftManagerLeaderCandidateStatus::BlockInChain;
            } else if !candidate.proposed_block_found {
                status = PbftManagerLeaderCandidateStatus::BlockMissingOrInvalid;
            } else {
                match candidate.block_validation_status {
                    PbftManagerLeaderBlockValidationStatus::AlreadyValid => {}
                    PbftManagerLeaderBlockValidationStatus::Validated => {
                        valid_blocks.push(PbftManagerLeaderValidBlockCommand {
                            period: candidate.period,
                            block_hash: candidate.block_hash,
                        });
                    }
                    PbftManagerLeaderBlockValidationStatus::Rejected => {
                        status = PbftManagerLeaderCandidateStatus::BlockMissingOrInvalid;
                    }
                    PbftManagerLeaderBlockValidationStatus::Unknown => {
                        return pbft_manager_candidate_plan_from_selection(
                            pbft_manager_leader_no_selection(
                                PbftManagerLeaderSelectionStatus::InvalidFact,
                                "PBFT_MANAGER_LEADER_UNKNOWN_BLOCK_VALIDATION_STATUS",
                            ),
                            valid_blocks,
                        );
                    }
                }
            }
        }

        selection_candidates.push(PbftManagerLeaderCandidateFact {
            vote_hash: candidate.vote_hash,
            block_hash: candidate.block_hash,
            period: candidate.period,
            credential: candidate.credential,
            voter_public_key: candidate.voter_public_key,
            weight,
            status,
            pivot_hash: candidate.pivot_hash,
        });
    }

    let selection = plan_pbft_manager_leader_selection(selection_candidates);
    pbft_manager_candidate_plan_from_selection(selection, valid_blocks)
}

/// Plans the next step of PBFT block validation.
///
/// The planner is a side-effect-free state machine: C++ supplies the latest
/// validation fact bundle, Rust requests the next live check, and C++ reports
/// that result back into the next call. The accepted/rejected outcome is
/// therefore Rust-owned even while live PBFT chain, FinalChain, reward-vote,
/// pillar, and DAG objects remain outside Rust.
#[must_use]
pub fn plan_pbft_manager_block_validation(
    fact: PbftManagerBlockValidationFact,
) -> PbftManagerBlockValidationPlan {
    if fact.pbft_chain_status == PbftManagerBlockValidationFactStatus::Unknown
        || fact.final_chain_hash_status == PbftManagerBlockValidationFactStatus::Unknown
        || fact.reward_votes_status == PbftManagerBlockValidationFactStatus::Unknown
        || fact.extra_data_status == PbftManagerBlockValidationFactStatus::Unknown
        || fact.pillar_block_status == PbftManagerBlockValidationFactStatus::Unknown
        || fact.dag_order_status == PbftManagerBlockValidationFactStatus::Unknown
        || fact.dag_weight_status == PbftManagerBlockValidationFactStatus::Unknown
    {
        return pbft_manager_block_validation_contract_error(
            "PBFT_MANAGER_BLOCK_VALIDATION_UNKNOWN_FACT_STATUS",
        );
    }

    match fact.pbft_chain_status {
        PbftManagerBlockValidationFactStatus::NotChecked => {
            return pbft_manager_block_validation_run_check(
                PbftManagerBlockValidationNextCheck::CheckPbftChain,
            );
        }
        PbftManagerBlockValidationFactStatus::Valid => {}
        PbftManagerBlockValidationFactStatus::Invalid
        | PbftManagerBlockValidationFactStatus::Missing => {
            return pbft_manager_block_validation_reject(
                PbftManagerBlockValidationStatus::PbftChainInvalid,
                "PBFT_MANAGER_BLOCK_VALIDATION_PBFT_CHAIN_INVALID",
            );
        }
        PbftManagerBlockValidationFactStatus::NotRequired
        | PbftManagerBlockValidationFactStatus::Unknown => {
            return pbft_manager_block_validation_contract_error(
                "PBFT_MANAGER_BLOCK_VALIDATION_PBFT_CHAIN_STATUS_INVALID",
            );
        }
    }

    match fact.final_chain_hash_status {
        PbftManagerBlockValidationFactStatus::NotChecked => {
            return pbft_manager_block_validation_run_check(
                PbftManagerBlockValidationNextCheck::ValidateFinalChainHash,
            );
        }
        PbftManagerBlockValidationFactStatus::Valid => {}
        PbftManagerBlockValidationFactStatus::Missing => {
            return PbftManagerBlockValidationPlan {
                action: PbftManagerBlockValidationAction::WaitForFinalization,
                status: PbftManagerBlockValidationStatus::FinalChainHashMissing,
                next_check: PbftManagerBlockValidationNextCheck::ValidateFinalChainHash,
                error_code: "PBFT_MANAGER_BLOCK_VALIDATION_FINAL_CHAIN_HASH_MISSING",
            };
        }
        PbftManagerBlockValidationFactStatus::Invalid => {
            return pbft_manager_block_validation_reject(
                PbftManagerBlockValidationStatus::FinalChainHashInvalid,
                "PBFT_MANAGER_BLOCK_VALIDATION_FINAL_CHAIN_HASH_INVALID",
            );
        }
        PbftManagerBlockValidationFactStatus::NotRequired
        | PbftManagerBlockValidationFactStatus::Unknown => {
            return pbft_manager_block_validation_contract_error(
                "PBFT_MANAGER_BLOCK_VALIDATION_FINAL_CHAIN_STATUS_INVALID",
            );
        }
    }

    match fact.reward_votes_status {
        PbftManagerBlockValidationFactStatus::NotChecked => {
            return pbft_manager_block_validation_run_check(
                PbftManagerBlockValidationNextCheck::CheckRewardVotes,
            );
        }
        PbftManagerBlockValidationFactStatus::Valid => {}
        PbftManagerBlockValidationFactStatus::Invalid
        | PbftManagerBlockValidationFactStatus::Missing => {
            return pbft_manager_block_validation_reject(
                PbftManagerBlockValidationStatus::RewardVotesInvalid,
                "PBFT_MANAGER_BLOCK_VALIDATION_REWARD_VOTES_INVALID",
            );
        }
        PbftManagerBlockValidationFactStatus::NotRequired
        | PbftManagerBlockValidationFactStatus::Unknown => {
            return pbft_manager_block_validation_contract_error(
                "PBFT_MANAGER_BLOCK_VALIDATION_REWARD_VOTES_STATUS_INVALID",
            );
        }
    }

    match fact.extra_data_status {
        PbftManagerBlockValidationFactStatus::NotChecked => {
            return pbft_manager_block_validation_run_check(
                PbftManagerBlockValidationNextCheck::ValidateExtraData,
            );
        }
        PbftManagerBlockValidationFactStatus::Valid => {}
        PbftManagerBlockValidationFactStatus::Invalid
        | PbftManagerBlockValidationFactStatus::Missing => {
            return pbft_manager_block_validation_reject(
                PbftManagerBlockValidationStatus::ExtraDataInvalid,
                "PBFT_MANAGER_BLOCK_VALIDATION_EXTRA_DATA_INVALID",
            );
        }
        PbftManagerBlockValidationFactStatus::NotRequired
        | PbftManagerBlockValidationFactStatus::Unknown => {
            return pbft_manager_block_validation_contract_error(
                "PBFT_MANAGER_BLOCK_VALIDATION_EXTRA_DATA_STATUS_INVALID",
            );
        }
    }

    if fact.pillar_block_required {
        match fact.pillar_block_status {
            PbftManagerBlockValidationFactStatus::NotChecked => {
                return pbft_manager_block_validation_run_check(
                    PbftManagerBlockValidationNextCheck::ValidatePillarBlock,
                );
            }
            PbftManagerBlockValidationFactStatus::Valid => {}
            PbftManagerBlockValidationFactStatus::Invalid
            | PbftManagerBlockValidationFactStatus::Missing => {
                return pbft_manager_block_validation_reject(
                    PbftManagerBlockValidationStatus::PillarBlockInvalid,
                    "PBFT_MANAGER_BLOCK_VALIDATION_PILLAR_BLOCK_INVALID",
                );
            }
            PbftManagerBlockValidationFactStatus::NotRequired
            | PbftManagerBlockValidationFactStatus::Unknown => {
                return pbft_manager_block_validation_contract_error(
                    "PBFT_MANAGER_BLOCK_VALIDATION_PILLAR_BLOCK_STATUS_INVALID",
                );
            }
        }
    } else if fact.pillar_block_status == PbftManagerBlockValidationFactStatus::NotChecked {
        // Normalize not-required checks so the C++ executor does not need to
        // report unused facts for non-pillar periods.
    } else if fact.pillar_block_status != PbftManagerBlockValidationFactStatus::NotRequired {
        return pbft_manager_block_validation_contract_error(
            "PBFT_MANAGER_BLOCK_VALIDATION_UNEXPECTED_PILLAR_BLOCK_STATUS",
        );
    }

    if fact.pivot_is_null || fact.dag_order_cached || !fact.dag_order_required {
        return pbft_manager_block_validation_accept();
    }

    match fact.dag_order_status {
        PbftManagerBlockValidationFactStatus::NotChecked => {
            return pbft_manager_block_validation_run_check(
                PbftManagerBlockValidationNextCheck::CheckDagOrder,
            );
        }
        PbftManagerBlockValidationFactStatus::Valid => {}
        PbftManagerBlockValidationFactStatus::Missing => {
            return pbft_manager_block_validation_reject(
                PbftManagerBlockValidationStatus::DagOrderMissing,
                "PBFT_MANAGER_BLOCK_VALIDATION_DAG_ORDER_MISSING",
            );
        }
        PbftManagerBlockValidationFactStatus::Invalid => {
            return pbft_manager_block_validation_reject(
                PbftManagerBlockValidationStatus::DagOrderInvalid,
                "PBFT_MANAGER_BLOCK_VALIDATION_DAG_ORDER_INVALID",
            );
        }
        PbftManagerBlockValidationFactStatus::NotRequired
        | PbftManagerBlockValidationFactStatus::Unknown => {
            return pbft_manager_block_validation_contract_error(
                "PBFT_MANAGER_BLOCK_VALIDATION_DAG_ORDER_STATUS_INVALID",
            );
        }
    }

    if !fact.dag_weight_check_required {
        return pbft_manager_block_validation_accept();
    }

    match fact.dag_weight_status {
        PbftManagerBlockValidationFactStatus::NotChecked => {
            pbft_manager_block_validation_run_check(
                PbftManagerBlockValidationNextCheck::CheckDagWeight,
            )
        }
        PbftManagerBlockValidationFactStatus::Valid => pbft_manager_block_validation_accept(),
        PbftManagerBlockValidationFactStatus::Invalid
        | PbftManagerBlockValidationFactStatus::Missing => pbft_manager_block_validation_reject(
            PbftManagerBlockValidationStatus::DagWeightInvalid,
            "PBFT_MANAGER_BLOCK_VALIDATION_DAG_WEIGHT_INVALID",
        ),
        PbftManagerBlockValidationFactStatus::NotRequired
        | PbftManagerBlockValidationFactStatus::Unknown => {
            pbft_manager_block_validation_contract_error(
                "PBFT_MANAGER_BLOCK_VALIDATION_DAG_WEIGHT_STATUS_INVALID",
            )
        }
    }
}

fn pbft_manager_block_validation_run_check(
    next_check: PbftManagerBlockValidationNextCheck,
) -> PbftManagerBlockValidationPlan {
    PbftManagerBlockValidationPlan {
        action: PbftManagerBlockValidationAction::RunCheck,
        status: PbftManagerBlockValidationStatus::Pending,
        next_check,
        error_code: "",
    }
}

fn pbft_manager_block_validation_accept() -> PbftManagerBlockValidationPlan {
    PbftManagerBlockValidationPlan {
        action: PbftManagerBlockValidationAction::Accept,
        status: PbftManagerBlockValidationStatus::Accepted,
        next_check: PbftManagerBlockValidationNextCheck::None,
        error_code: "",
    }
}

fn pbft_manager_block_validation_reject(
    status: PbftManagerBlockValidationStatus,
    error_code: &'static str,
) -> PbftManagerBlockValidationPlan {
    PbftManagerBlockValidationPlan {
        action: PbftManagerBlockValidationAction::Reject,
        status,
        next_check: PbftManagerBlockValidationNextCheck::None,
        error_code,
    }
}

fn pbft_manager_block_validation_contract_error(
    error_code: &'static str,
) -> PbftManagerBlockValidationPlan {
    PbftManagerBlockValidationPlan {
        action: PbftManagerBlockValidationAction::ContractError,
        status: PbftManagerBlockValidationStatus::InvalidBridgeFacts,
        next_check: PbftManagerBlockValidationNextCheck::None,
        error_code,
    }
}

fn pbft_manager_candidate_admission_contract_error(
    error_code: &'static str,
) -> PbftManagerCandidateAdmissionPlan {
    PbftManagerCandidateAdmissionPlan {
        action: PbftManagerCandidateAdmissionAction::ContractError,
        status: PbftManagerCandidateAdmissionStatus::InvalidBridgeFacts,
        mark_valid: false,
        error_code,
    }
}

fn pbft_manager_candidate_plan_from_selection(
    selection: PbftManagerLeaderSelectionPlan,
    valid_blocks: Vec<PbftManagerLeaderValidBlockCommand>,
) -> PbftManagerLeaderCandidatePlan {
    PbftManagerLeaderCandidatePlan {
        status: selection.status,
        selected: selection.selected,
        selected_vote_hash: selection.selected_vote_hash,
        selected_block_hash: selection.selected_block_hash,
        selected_period: selection.selected_period,
        selected_from_null_anchor: selection.selected_from_null_anchor,
        valid_blocks,
        error_code: selection.error_code,
    }
}

fn pbft_manager_leader_selected(
    candidate: PbftManagerLeaderCandidateFact,
    selected_from_null_anchor: bool,
) -> PbftManagerLeaderSelectionPlan {
    PbftManagerLeaderSelectionPlan {
        status: PbftManagerLeaderSelectionStatus::Selected,
        selected: true,
        selected_vote_hash: candidate.vote_hash,
        selected_block_hash: candidate.block_hash,
        selected_period: candidate.period,
        selected_from_null_anchor,
        error_code: "",
    }
}

fn pbft_manager_leader_no_selection(
    status: PbftManagerLeaderSelectionStatus,
    error_code: &'static str,
) -> PbftManagerLeaderSelectionPlan {
    PbftManagerLeaderSelectionPlan {
        status,
        selected: false,
        selected_vote_hash: H256::zero(),
        selected_block_hash: H256::zero(),
        selected_period: 0,
        selected_from_null_anchor: false,
        error_code,
    }
}

fn keccak256(data: &[u8]) -> H256 {
    let mut output = [0_u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(data);
    hasher.finalize(&mut output);
    H256::from(output)
}

impl PbftManagerRuntimeStateCode {
    /// Stable bridge code for the state.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ValueProposal => 0,
            Self::Filter => 1,
            Self::Certify => 2,
            Self::Finish => 3,
            Self::FinishPolling => 4,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::ValueProposal,
            1 => Self::Filter,
            2 => Self::Certify,
            3 => Self::Finish,
            4 => Self::FinishPolling,
            _ => Self::Unknown,
        }
    }
}

/// Runtime status for one Rust-owned PBFT manager tick session.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerRuntimeStatus {
    /// Session is ready to execute or report the next action.
    Active,
    /// Session completed all actions.
    Complete,
    /// The tick facts were rejected before execution.
    RejectedTick,
    /// Reported action does not match the current cursor.
    ActionMismatch,
    /// The C++ executor reported action failure.
    ActionFailed,
    /// The report was malformed for the current action.
    InvalidReport,
    /// Internal invariant or unknown bridge-code failure.
    ContractError,
    /// Unknown bridge status.
    Unknown,
}

impl PbftManagerRuntimeStatus {
    /// Stable bridge code for the status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Active => 0,
            Self::Complete => 1,
            Self::RejectedTick => 2,
            Self::ActionMismatch => 3,
            Self::ActionFailed => 4,
            Self::InvalidReport => 5,
            Self::ContractError => 255,
            Self::Unknown => 254,
        }
    }
}

/// Stable action codes for one PBFT manager daemon tick.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerRuntimeAction {
    /// Drain synced PBFT blocks into the local chain path.
    ProcessSyncedPbftBlocks,
    /// Broadcast or rebroadcast votes according to existing C++ timers.
    MaybeBroadcastVotes,
    /// Try to push a cert-voted PBFT block into the chain.
    TryPushCertVotesBlock,
    /// Try to advance to a higher round from next-vote facts.
    TryAdvanceRound,
    /// Reset consensus to a Rust-selected target round.
    ResetConsensus,
    /// Sleep briefly when the node has no eligible wallet for active steps.
    SleepIneligiblePollingInterval,
    /// Execute value proposal behavior.
    RunValueProposal,
    /// Transition from value proposal to filter state.
    TransitionToFilter,
    /// Execute filtering / leader-identification behavior.
    RunFilter,
    /// Transition from filter to certify state.
    TransitionToCertify,
    /// Execute certifying behavior.
    RunCertify,
    /// Transition from certify to first finish state.
    TransitionToFinish,
    /// Delay certify polling without changing state.
    DelayCertifyPoll,
    /// Execute first finish behavior.
    RunFirstFinish,
    /// Transition from first finish to finish-polling state.
    TransitionToFinishPolling,
    /// Execute second finish / polling behavior.
    RunSecondFinish,
    /// Loop from finish-polling back to first finish.
    LoopBackFinish,
    /// Delay finish-polling without changing state.
    DelayFinishPoll,
    /// Sleep until the next planned step time.
    SleepUntilNextStep,
    /// Unknown bridge action code.
    Unknown,
}

impl PbftManagerRuntimeAction {
    /// Stable bridge code for the action.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ProcessSyncedPbftBlocks => 0,
            Self::MaybeBroadcastVotes => 1,
            Self::TryPushCertVotesBlock => 2,
            Self::TryAdvanceRound => 3,
            Self::SleepIneligiblePollingInterval => 4,
            Self::RunValueProposal => 5,
            Self::TransitionToFilter => 6,
            Self::RunFilter => 7,
            Self::TransitionToCertify => 8,
            Self::RunCertify => 9,
            Self::TransitionToFinish => 10,
            Self::DelayCertifyPoll => 11,
            Self::RunFirstFinish => 12,
            Self::TransitionToFinishPolling => 13,
            Self::RunSecondFinish => 14,
            Self::LoopBackFinish => 15,
            Self::DelayFinishPoll => 16,
            Self::SleepUntilNextStep => 17,
            Self::ResetConsensus => 18,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge action code.
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::ProcessSyncedPbftBlocks),
            1 => Some(Self::MaybeBroadcastVotes),
            2 => Some(Self::TryPushCertVotesBlock),
            3 => Some(Self::TryAdvanceRound),
            4 => Some(Self::SleepIneligiblePollingInterval),
            5 => Some(Self::RunValueProposal),
            6 => Some(Self::TransitionToFilter),
            7 => Some(Self::RunFilter),
            8 => Some(Self::TransitionToCertify),
            9 => Some(Self::RunCertify),
            10 => Some(Self::TransitionToFinish),
            11 => Some(Self::DelayCertifyPoll),
            12 => Some(Self::RunFirstFinish),
            13 => Some(Self::TransitionToFinishPolling),
            14 => Some(Self::RunSecondFinish),
            15 => Some(Self::LoopBackFinish),
            16 => Some(Self::DelayFinishPoll),
            17 => Some(Self::SleepUntilNextStep),
            18 => Some(Self::ResetConsensus),
            _ => Some(Self::Unknown),
        }
    }
}

/// Stable result code for one C++-executed manager action.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerRuntimeActionResultCode {
    /// Action succeeded and session should continue normally.
    NoProgressContinue,
    /// Action made progress and the manager loop must restart immediately.
    ProgressRestartLoop,
    /// State action completed.
    StateActionDone,
    /// State transition completed.
    TransitionApplied,
    /// Sleep action completed.
    SleepApplied,
    /// C++ executor reported an error.
    ExecutorError,
    /// Unknown bridge result.
    Unknown,
}

impl PbftManagerRuntimeActionResultCode {
    /// Stable bridge code for action report results.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::NoProgressContinue => 0,
            Self::ProgressRestartLoop => 1,
            Self::StateActionDone => 2,
            Self::TransitionApplied => 3,
            Self::SleepApplied => 4,
            Self::ExecutorError => 255,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge result code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::NoProgressContinue,
            1 => Self::ProgressRestartLoop,
            2 => Self::StateActionDone,
            3 => Self::TransitionApplied,
            4 => Self::SleepApplied,
            255 => Self::ExecutorError,
            _ => Self::Unknown,
        }
    }
}

/// C++-originated facts for one PBFT manager daemon tick.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftManagerRuntimeTickFact {
    /// Monotonic caller-local tick id for telemetry.
    pub tick_id: u64,
    /// Current PBFT manager state.
    pub state: PbftManagerRuntimeStateCode,
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub round: u64,
    /// Current PBFT step.
    pub step: u64,
    /// Whether the network handle is currently available.
    pub network_available: bool,
    /// Whether the network reports PBFT sync mode.
    pub network_pbft_syncing: bool,
    /// Initial eligibility snapshot for telemetry. The runtime branch uses the
    /// post-prestate value reported by C++ after `TryAdvanceRound`.
    pub has_eligible_wallet: bool,
}

/// Stable action-intent codes for deterministic PBFT state actions.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerStateActionIntent {
    /// No consensus-side work should be executed for this state action.
    Noop,
    /// Build and propose a fresh PBFT block for the current period/round.
    ProposeNewBlock,
    /// Re-propose the previous round's 2t+1 next-voted value.
    ReproposePreviousRoundNextValue,
    /// Identify the current round leader block and soft-vote it if present.
    IdentifyLeaderAndSoftVote,
    /// Soft-vote the previous round's 2t+1 next-voted value.
    SoftVotePreviousRoundNextValue,
    /// Cert-vote the current round's 2t+1 soft-voted value.
    CertVoteCurrentSoftValue,
    /// Move from certify polling to the finish state.
    GoFinish,
    /// Next-vote the block this node cert-voted in the current round.
    NextVoteCertVotedBlock,
    /// Next-vote the null block hash.
    NextVoteNullBlock,
    /// Next-vote the previous round's 2t+1 next-voted value.
    NextVotePreviousRoundValue,
    /// Next-vote the current round's 2t+1 soft-voted value.
    NextVoteCurrentSoftValue,
}

impl PbftManagerStateActionIntent {
    /// Stable bridge code for the state-action intent.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Noop => 0,
            Self::ProposeNewBlock => 1,
            Self::ReproposePreviousRoundNextValue => 2,
            Self::IdentifyLeaderAndSoftVote => 3,
            Self::SoftVotePreviousRoundNextValue => 4,
            Self::CertVoteCurrentSoftValue => 5,
            Self::GoFinish => 6,
            Self::NextVoteCertVotedBlock => 7,
            Self::NextVoteNullBlock => 8,
            Self::NextVotePreviousRoundValue => 9,
            Self::NextVoteCurrentSoftValue => 10,
        }
    }

    /// Decodes a stable bridge state-action code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::ProposeNewBlock,
            2 => Self::ReproposePreviousRoundNextValue,
            3 => Self::IdentifyLeaderAndSoftVote,
            4 => Self::SoftVotePreviousRoundNextValue,
            5 => Self::CertVoteCurrentSoftValue,
            6 => Self::GoFinish,
            7 => Self::NextVoteCertVotedBlock,
            8 => Self::NextVoteNullBlock,
            9 => Self::NextVotePreviousRoundValue,
            10 => Self::NextVoteCurrentSoftValue,
            _ => Self::Noop,
        }
    }
}

/// Stable status codes for PBFT manager state-action planning.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerStateActionStatus {
    /// The plan is usable by the C++ executor.
    Ready,
    /// The supplied state is unknown or unsupported.
    InvalidState,
    /// The supplied fact bundle is internally inconsistent.
    InvalidFact,
}

impl PbftManagerStateActionStatus {
    /// Stable bridge code for the state-action status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::InvalidState => 1,
            Self::InvalidFact => 2,
        }
    }
}

/// C++-originated facts for one PBFT manager state action.
///
/// The fact bundle is intentionally compact and contains only deterministic
/// branch inputs. C++ remains responsible for sourcing those facts from live
/// vote/proposed-block sidecars, executing returned intents, materializing
/// blocks and votes, writing storage, and emitting network effects.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStateActionFact {
    /// State being executed.
    pub state: PbftManagerRuntimeStateCode,
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub round: u64,
    /// Current PBFT step.
    pub step: u64,
    /// Elapsed milliseconds in the current round.
    pub elapsed_round_ms: u64,
    /// PBFT deadline for the current round in milliseconds.
    pub deadline_ms: u64,
    /// Current round lambda in milliseconds.
    pub current_round_lambda_ms: u64,
    /// Polling interval used by the legacy manager loop.
    pub polling_interval_ms: u64,
    /// Whether the previous round has 2t+1 next votes for null.
    pub has_previous_round_next_null: bool,
    /// Whether the previous round has 2t+1 next votes for a block value.
    pub has_previous_round_next_value: bool,
    /// Previous round 2t+1 next-voted block value, when present.
    pub previous_round_next_value_hash: [u8; 32],
    /// Whether the current round has 2t+1 soft votes for a block value.
    pub has_current_round_soft_value: bool,
    /// Current round 2t+1 soft-voted block value, when present.
    pub current_round_soft_value_hash: [u8; 32],
    /// Whether this node already cert-voted a block in this round.
    pub has_cert_voted_block: bool,
    /// Current round cert-voted block hash, when present.
    pub cert_voted_block_hash: [u8; 32],
    /// Whether this node already emitted a next vote for a soft-voted value.
    pub already_next_voted_value: bool,
    /// Whether this node already emitted a null-block next vote.
    pub already_next_voted_null: bool,
}

/// Side-effect-free PBFT manager state-action plan.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStateActionPlan {
    /// Planning status.
    pub status: PbftManagerStateActionStatus,
    /// Primary action intent for the C++ executor.
    pub primary_intent: PbftManagerStateActionIntent,
    /// Hash argument for the primary intent, if applicable.
    pub primary_hash: [u8; 32],
    /// Secondary action intent for states that can emit two vote attempts.
    pub secondary_intent: PbftManagerStateActionIntent,
    /// Hash argument for the secondary intent, if applicable.
    pub secondary_hash: [u8; 32],
    /// Planned value for `go_finish_state_`.
    pub go_finish_state: bool,
    /// Planned value for `loop_back_finish_state_`.
    pub loop_back_finish_state: bool,
    /// Stable error detail for rejected facts.
    pub error_code: String,
}

/// Stable transition-kind codes for PBFT manager cursor mutation planning.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerTransitionKind {
    /// Reset the consensus cursor for a new round.
    ResetConsensus,
    /// Move from value-proposal to filtering.
    ToFilter,
    /// Move from filtering to certifying.
    ToCertify,
    /// Move from certifying to first finish.
    ToFinish,
    /// Move from first finish to finish polling.
    ToFinishPolling,
    /// Loop from finish polling back to first finish.
    LoopBackFinish,
    /// Delay certify polling without changing phase.
    DelayCertifyPoll,
    /// Delay finish polling without changing phase.
    DelayFinishPoll,
    /// Unknown bridge transition code.
    Unknown,
}

impl PbftManagerTransitionKind {
    /// Stable bridge code for this transition kind.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ResetConsensus => 0,
            Self::ToFilter => 1,
            Self::ToCertify => 2,
            Self::ToFinish => 3,
            Self::ToFinishPolling => 4,
            Self::LoopBackFinish => 5,
            Self::DelayCertifyPoll => 6,
            Self::DelayFinishPoll => 7,
            Self::Unknown => 254,
        }
    }

    /// Decodes a stable bridge transition code.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::ResetConsensus,
            1 => Self::ToFilter,
            2 => Self::ToCertify,
            3 => Self::ToFinish,
            4 => Self::ToFinishPolling,
            5 => Self::LoopBackFinish,
            6 => Self::DelayCertifyPoll,
            7 => Self::DelayFinishPoll,
            _ => Self::Unknown,
        }
    }
}

/// Stable status codes for PBFT manager transition planning.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerTransitionStatus {
    /// The transition plan is usable by the C++ executor.
    Ready,
    /// The supplied transition kind is unknown.
    InvalidKind,
    /// The supplied facts are internally inconsistent.
    InvalidFact,
}

impl PbftManagerTransitionStatus {
    /// Stable bridge code for the transition status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::InvalidKind => 1,
            Self::InvalidFact => 2,
        }
    }
}

/// C++-originated facts for one PBFT manager cursor/status transition.
///
/// The fact bundle contains only scalar state, timing, and already-sourced
/// network vote progress. Rust decides the resulting manager cursor, lambda,
/// next-step deadline, and manager-status reset intents. C++ remains the
/// executor for storage writes, VoteManager side effects, live status fields,
/// timestamps, and compatibility logging.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerTransitionFact {
    /// Transition kind requested by the runtime cursor.
    pub kind: PbftManagerTransitionKind,
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub round: u64,
    /// Current PBFT step.
    pub step: u64,
    /// Target round for `ResetConsensus`; ignored by phase transitions.
    pub target_round: u64,
    /// Current round lambda before the transition.
    pub current_round_lambda_ms: u64,
    /// Lambda calculated for the target round under Cacti.
    pub target_round_lambda_ms: u64,
    /// Genesis/default lambda used before Cacti and for exponential reset.
    pub default_lambda_ms: u64,
    /// Maximum exponential lambda.
    pub max_exponential_lambda_ms: u64,
    /// Odd step where exponential backoff starts.
    pub max_steps: u64,
    /// Greatest network t+1 next-voting step already sourced by C++.
    pub network_next_voting_step: u64,
    /// PBFT deadline for the current round in milliseconds.
    pub deadline_ms: u64,
    /// Polling interval used by finish polling.
    pub polling_interval_ms: u64,
    /// Current `next_step_time_ms_`.
    pub next_step_time_ms: u64,
    /// Whether the target period is on the Cacti hardfork.
    pub cacti_hardfork: bool,
    /// Whether a cert-voted block sidecar exists and may need removal.
    pub has_cert_voted_block: bool,
    /// Whether an executed PBFT block flag is set and requires executor reset.
    pub executed_pbft_block: bool,
}

/// Side-effect-free plan for one PBFT manager cursor/status transition.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerTransitionPlan {
    /// Planning status.
    pub status: PbftManagerTransitionStatus,
    /// Transition kind echoed back for executor validation.
    pub kind: PbftManagerTransitionKind,
    /// Planned PBFT state after the transition.
    pub new_state: PbftManagerRuntimeStateCode,
    /// Planned round after the transition.
    pub new_round: u64,
    /// Planned step after the transition.
    pub new_step: u64,
    /// Planned current-round lambda in milliseconds.
    pub current_round_lambda_ms: u64,
    /// Planned next-step deadline in milliseconds.
    pub next_step_time_ms: u64,
    /// Persist the planned round field.
    pub persist_round: bool,
    /// Persist the planned step field.
    pub persist_step: bool,
    /// Reset next-voted manager status bits and live flags.
    pub reset_next_voted_statuses: bool,
    /// Remove the saved cert-voted block if present.
    pub remove_cert_voted_block: bool,
    /// Clear local own-vote records through the VoteManager executor.
    pub clear_own_votes: bool,
    /// Clear current-round broadcast bookkeeping.
    pub clear_broadcasted_votes: bool,
    /// Reset current-round broadcast counters.
    pub reset_broadcast_counters: bool,
    /// Reset the executed-block manager status after period finalization.
    pub reset_executed_block_status: bool,
    /// Update the VoteManager period/round executor boundary.
    pub set_vote_manager_period_round: bool,
    /// Reset current round start time in C++.
    pub reset_current_round_start: bool,
    /// Reset second-finish polling start time in C++.
    pub reset_second_finish_start: bool,
    /// Set the certify-step log flag.
    pub print_cert_step_info: bool,
    /// Set the second-finish-step log flag.
    pub print_second_finish_step_info: bool,
    /// Stable error detail for rejected facts.
    pub error_code: String,
}

/// Durable storage result for one PBFT manager transition commit.
///
/// Inputs:
/// - Produced only by Rust-owned PBFT manager transition storage helpers.
///
/// Outputs:
/// - `status` records whether the storage commit applied or was rejected.
/// - `applied_writes` reports the number of manager/status/vote rows written
///   or removed before commit.
/// - `error_code` is stable bridge-facing detail for rejected plans, overflow,
///   storage write failure, or commit failure.
///
/// Invariants and edge behavior:
/// - Rejected results are returned before the Rust runtime cursor is advanced.
/// - Rejected write batches are dropped by ownership; callers must not update
///   C++ mirrors or runtime snapshots for rejected results.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerTransitionStorageResult {
    /// Stable storage-apply status.
    pub status: PbftManagerTransitionStorageStatus,
    /// Number of durable writes/deletes requested by the accepted commit.
    pub applied_writes: u64,
    /// Stable rejection detail, empty for applied commits.
    pub error_code: String,
}

/// Stable storage-apply status for PBFT manager transition commits.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerTransitionStorageStatus {
    /// The Rust-owned storage batch committed.
    Applied,
    /// The plan or storage operation was rejected without advancing runtime
    /// state.
    Rejected,
}

impl PbftManagerTransitionStorageStatus {
    /// Stable bridge code for the transition storage result.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Applied => 0,
            Self::Rejected => 1,
        }
    }
}

/// Stable status codes for PBFT manager runtime startup restore.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftManagerStartupRestoreStatus {
    /// Startup facts are valid and the runtime snapshot is usable.
    Ready,
    /// Startup facts are internally inconsistent or represent corrupted
    /// persisted manager state.
    InvalidFact,
}

impl PbftManagerStartupRestoreStatus {
    /// Stable bridge code for the startup restore status.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::InvalidFact => 1,
        }
    }
}

/// Persisted and configuration facts used to restore the PBFT manager runtime.
///
/// The fact bundle is deliberately scalar-only. Storage and bridge code read
/// persisted DB values, then Rust decides the normalized PBFT cursor and live
/// startup flags without materializing PBFT blocks, votes, network handles, or
/// FinalChain objects.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerStartupRestoreFact {
    /// Current PBFT period at startup.
    pub current_period: u64,
    /// Persisted manager round, defaulted by storage compatibility to `1` when
    /// absent.
    pub persisted_round: u64,
    /// Persisted manager step, defaulted by storage compatibility to `1` when
    /// absent.
    pub persisted_step: u64,
    /// Whether the Cacti dynamic-lambda rules are active for
    /// `current_period - 1`.
    pub cacti_active_at_chain_size: bool,
    /// Persisted rounds-count dynamic-lambda accumulator.
    pub rounds_count_dynamic_lambda: u32,
    /// Persisted dynamic lambda manager field, defaulted by storage
    /// compatibility to `1` when absent.
    pub persisted_dynamic_lambda_ms: u32,
    /// Genesis PBFT lambda used before Cacti.
    pub genesis_lambda_ms: u32,
    /// Cacti maximum lambda used as live default before any finalized Cacti
    /// period has saved a dynamic lambda.
    pub cacti_lambda_max_ms: u32,
    /// Cacti non-round-one lambda.
    pub cacti_lambda_default_ms: u32,
    /// Persisted executed-block manager status.
    pub executed_pbft_block: bool,
    /// Persisted next-voted-value manager status.
    pub already_next_voted_value: bool,
    /// Persisted next-voted-null manager status.
    pub already_next_voted_null: bool,
}

/// Storage-backed PBFT manager startup configuration.
///
/// Purpose:
/// - Carries only non-storage startup configuration into the native Rust
///   storage restore path. Persisted manager fields and statuses are read
///   directly from `rustaxa-storage` by
///   `create_pbft_manager_runtime_from_storage`.
///
/// Inputs:
/// - `current_period` is the PBFT period observed by the C++ compatibility
///   shell at startup.
/// - Cacti and lambda fields are configuration facts that are not stored in
///   the PBFT manager storage columns.
///
/// Invariants and edge behavior:
/// - Lambda values must be nonzero. Invalid or corrupted persisted storage
///   facts are rejected with stable error labels from
///   `restore_pbft_manager_runtime`.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftManagerStorageStartupFact {
    /// Current PBFT period at startup.
    pub current_period: u64,
    /// Whether the Cacti dynamic-lambda rules are active for
    /// `current_period - 1`.
    pub cacti_active_at_chain_size: bool,
    /// Genesis PBFT lambda used before Cacti.
    pub genesis_lambda_ms: u32,
    /// Cacti maximum lambda used as live default before any finalized Cacti
    /// period has saved a dynamic lambda.
    pub cacti_lambda_max_ms: u32,
    /// Cacti non-round-one lambda.
    pub cacti_lambda_default_ms: u32,
}

/// Runtime cursor and live scalar facts restored for the PBFT manager shim.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntimeSnapshot {
    /// Restore/apply status.
    pub status: PbftManagerStartupRestoreStatus,
    /// Current PBFT manager state.
    pub state: PbftManagerRuntimeStateCode,
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub round: u64,
    /// Current PBFT step.
    pub step: u64,
    /// Current-round lambda in milliseconds.
    pub current_round_lambda_ms: u64,
    /// Next-step deadline in milliseconds.
    pub next_step_time_ms: u64,
    /// Live dynamic-lambda accumulator restored from storage.
    pub rounds_count_dynamic_lambda: u32,
    /// Live dynamic lambda in milliseconds.
    pub dynamic_lambda_ms: u32,
    /// Live executed-block flag.
    pub executed_pbft_block: bool,
    /// Live next-voted-value flag.
    pub already_next_voted_value: bool,
    /// Live next-voted-null flag.
    pub already_next_voted_null: bool,
    /// Whether startup normalized persisted step and must persist the new step
    /// before C++ mirrors are updated.
    pub persist_normalized_step: bool,
    /// Whether C++ should reset the second-finish polling timestamp.
    pub reset_second_finish_start: bool,
    /// Stable error detail for rejected startup facts.
    pub error_code: String,
}

/// Long-lived PBFT manager runtime cursor owned by Rust.
///
/// This runtime owns the scalar PBFT manager cursor restored from storage and
/// updated after accepted transition storage commits. It does not own timers,
/// network effects, FinalChain/EVM execution, or live C++ PBFT object
/// materialization.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntime {
    snapshot: PbftManagerRuntimeSnapshot,
}

/// Storage-backed facts for replaying one finalized period during PBFT manager startup.
///
/// Inputs:
/// - Loaded from `rustaxa-storage` by `load_pbft_manager_startup_replay_period`.
///
/// Outputs:
/// - `period_data_rlp` is the canonical legacy `PeriodData` payload.
/// - `finalized_dag_hashes` preserves the finalized DAG block order encoded in
///   the period data.
/// - `period_lambda` is the closest persisted dynamic lambda when requested by
///   the startup replay path.
///
/// Invariants and edge behavior:
/// - `found = false` means no period data was present; all payload fields are
///   empty/default.
/// - Malformed period data returns an error rather than falling back to C++
///   storage decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbftManagerStartupReplayPeriod {
    /// Whether the requested period data exists in storage.
    pub found: bool,
    /// Canonical legacy `PeriodData` RLP bytes for C++ temporary sidecar materialization.
    pub period_data_rlp: Vec<u8>,
    /// Finalized DAG block hashes in persisted order.
    pub finalized_dag_hashes: Vec<H256>,
    /// Closest persisted dynamic lambda for this period, when requested and present.
    pub period_lambda: Option<u32>,
}

impl PbftManagerRuntime {
    /// Creates a runtime from an already accepted startup snapshot.
    pub fn new(snapshot: PbftManagerRuntimeSnapshot) -> Self {
        Self { snapshot }
    }

    /// Returns the current Rust-owned scalar snapshot.
    pub fn snapshot(&self) -> PbftManagerRuntimeSnapshot {
        self.snapshot.clone()
    }

    /// Advances the Rust-owned scalar cursor after transition storage commits.
    ///
    /// The caller must only invoke this after the corresponding Rust storage
    /// batch has been committed. Rejected plans are ignored so storage failure
    /// cannot move the in-memory Rust cursor ahead of durable state.
    pub fn apply_committed_transition(&mut self, plan: &PbftManagerTransitionPlan) {
        if plan.status != PbftManagerTransitionStatus::Ready {
            return;
        }

        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.state = plan.new_state;
        self.snapshot.round = plan.new_round;
        self.snapshot.step = plan.new_step;
        self.snapshot.current_round_lambda_ms = plan.current_round_lambda_ms;
        self.snapshot.next_step_time_ms = plan.next_step_time_ms;
        self.snapshot.persist_normalized_step = false;
        self.snapshot.reset_second_finish_start = plan.reset_second_finish_start;
        self.snapshot.error_code.clear();
        if plan.reset_next_voted_statuses {
            self.snapshot.already_next_voted_value = false;
            self.snapshot.already_next_voted_null = false;
        }
    }

    /// Records the delayed executed-block status reset after persistence.
    ///
    /// Reset-consensus plans keep the legacy wait-for-finalization ordering for
    /// the durable `ExecutedBlock` manager status. The bridge calls this only
    /// after that Rust storage write succeeds, so later C++ mirror updates are
    /// sourced from an authoritative Rust runtime snapshot instead of a stale
    /// pre-reset flag.
    pub fn apply_committed_executed_block_reset(&mut self) {
        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.executed_pbft_block = false;
        self.snapshot.error_code.clear();
    }

    /// Records a committed next-vote status after Rust storage persistence.
    ///
    /// Inputs:
    /// - `status`: stable PBFT manager status id for the next-voted soft value
    ///   or next-voted null-block-hash flag.
    ///
    /// Outputs:
    /// - Updates the matching runtime snapshot flag and clears the restore
    ///   error code.
    ///
    /// Invariants and edge behavior:
    /// - Callers must persist the matching status row before invoking this
    ///   method, so the long-lived runtime never advances ahead of durable
    ///   storage.
    /// - Unsupported status ids are ignored here because
    ///   `apply_next_voted_status_storage` rejects them before the bridge calls
    ///   this method.
    pub fn apply_committed_next_voted_status(&mut self, status: u8) {
        match status {
            PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE => {
                self.snapshot.already_next_voted_value = true;
            }
            PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH => {
                self.snapshot.already_next_voted_null = true;
            }
            _ => return,
        }
        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.error_code.clear();
    }

    /// Records a committed PBFT manager cursor field after Rust storage persistence.
    ///
    /// Inputs:
    /// - `field`: stable PBFT manager field id for round or step.
    /// - `value`: durable cursor value that was just written to storage.
    ///
    /// Outputs:
    /// - Updates the matching runtime snapshot field and clears the restore
    ///   error code.
    ///
    /// Invariants and edge behavior:
    /// - Callers must persist the matching field row before invoking this
    ///   method, so the long-lived runtime never advances ahead of durable
    ///   storage.
    /// - Unsupported fields are ignored here because
    ///   `apply_pbft_manager_cursor_field_storage` rejects them before the
    ///   bridge calls this method.
    pub fn apply_committed_cursor_field(&mut self, field: u8, value: u32) {
        match field {
            PBFT_MGR_FIELD_ROUND => self.snapshot.round = u64::from(value),
            PBFT_MGR_FIELD_STEP => self.snapshot.step = u64::from(value),
            _ => return,
        }
        self.snapshot.status = PbftManagerStartupRestoreStatus::Ready;
        self.snapshot.error_code.clear();
    }
}

fn reject_startup_restore(error_code: &str) -> PbftManagerRuntimeSnapshot {
    PbftManagerRuntimeSnapshot {
        status: PbftManagerStartupRestoreStatus::InvalidFact,
        state: PbftManagerRuntimeStateCode::Unknown,
        period: 0,
        round: 0,
        step: 0,
        current_round_lambda_ms: 0,
        next_step_time_ms: 0,
        rounds_count_dynamic_lambda: 0,
        dynamic_lambda_ms: 0,
        executed_pbft_block: false,
        already_next_voted_value: false,
        already_next_voted_null: false,
        persist_normalized_step: false,
        reset_second_finish_start: false,
        error_code: error_code.to_string(),
    }
}

/// Restores the Rust-owned PBFT manager runtime cursor from persisted facts.
///
/// The restored snapshot mirrors legacy startup semantics: missing round/step
/// default to one, steps below four restart in first-finish at step four, even
/// steps restart in first-finish, and odd steps restart in finish-polling. Cacti
/// dynamic lambda is restored from the persisted manager field after at least
/// one Cacti period has finalized; a default `1` value in that case is rejected
/// as corrupted storage to preserve the legacy safety check.
pub fn restore_pbft_manager_runtime(
    fact: PbftManagerStartupRestoreFact,
) -> PbftManagerRuntimeSnapshot {
    if fact.current_period == 0 || fact.persisted_round == 0 || fact.persisted_step == 0 {
        return reject_startup_restore("PBFT_MANAGER_STARTUP_INVALID_CURSOR");
    }
    if fact.genesis_lambda_ms == 0
        || fact.cacti_lambda_max_ms == 0
        || fact.cacti_lambda_default_ms == 0
    {
        return reject_startup_restore("PBFT_MANAGER_STARTUP_INVALID_LAMBDA_CONFIG");
    }

    let chain_size = fact.current_period.saturating_sub(1);
    let dynamic_lambda_ms = if fact.cacti_active_at_chain_size {
        if chain_size >= 1 {
            if fact.persisted_dynamic_lambda_ms == 1 {
                return reject_startup_restore("PBFT_MANAGER_STARTUP_MISSING_DYNAMIC_LAMBDA");
            }
            fact.persisted_dynamic_lambda_ms
        } else {
            fact.cacti_lambda_max_ms
        }
    } else {
        fact.cacti_lambda_max_ms
    };

    let current_round_lambda_ms = if fact.cacti_active_at_chain_size {
        if fact.persisted_round == 1 {
            dynamic_lambda_ms
        } else {
            fact.cacti_lambda_default_ms
        }
    } else {
        fact.genesis_lambda_ms
    };

    let (state, step, persist_normalized_step, reset_second_finish_start) =
        if fact.persisted_round == 1 && fact.persisted_step == 1 {
            (PbftManagerRuntimeStateCode::ValueProposal, 1, false, false)
        } else if fact.persisted_step < 4 {
            (PbftManagerRuntimeStateCode::Finish, 4, true, false)
        } else if fact.persisted_step % 2 == 0 {
            (
                PbftManagerRuntimeStateCode::Finish,
                fact.persisted_step,
                false,
                false,
            )
        } else {
            (
                PbftManagerRuntimeStateCode::FinishPolling,
                fact.persisted_step,
                false,
                true,
            )
        };

    PbftManagerRuntimeSnapshot {
        status: PbftManagerStartupRestoreStatus::Ready,
        state,
        period: fact.current_period,
        round: fact.persisted_round,
        step,
        current_round_lambda_ms: u64::from(current_round_lambda_ms),
        next_step_time_ms: 0,
        rounds_count_dynamic_lambda: if fact.cacti_active_at_chain_size {
            fact.rounds_count_dynamic_lambda
        } else {
            0
        },
        dynamic_lambda_ms,
        executed_pbft_block: fact.executed_pbft_block,
        already_next_voted_value: fact.already_next_voted_value,
        already_next_voted_null: fact.already_next_voted_null,
        persist_normalized_step,
        reset_second_finish_start,
        error_code: String::new(),
    }
}

/// Creates a PBFT manager runtime from `rustaxa-storage` directly.
///
/// Purpose:
/// - Makes `rustaxa-consensus` the owner of PBFT manager startup storage
///   reads and normalization. The bridge may temporarily pass the shared
///   storage handle, but it no longer decides which storage rows form the
///   runtime snapshot.
///
/// Inputs:
/// - `storage` is the native Rust storage module.
/// - `fact` contains only live/config facts that are not stored in PBFT manager
///   columns.
///
/// Outputs:
/// - A Rust-owned PBFT manager runtime seeded from durable storage.
///
/// Invariants and edge behavior:
/// - Missing round/step/lambda fields preserve legacy compatibility defaults.
/// - If persisted step normalization is required, the normalized step is
///   written through `rustaxa-storage` before the returned runtime clears the
///   `persist_normalized_step` flag.
/// - Invalid startup facts return a stable error label and do not fall back to
///   C++ storage behavior.
pub fn create_pbft_manager_runtime_from_storage(
    storage: &Storage,
    fact: PbftManagerStorageStartupFact,
) -> Result<PbftManagerRuntime> {
    let pbft = storage.pbft();
    let mut snapshot = restore_pbft_manager_runtime(PbftManagerStartupRestoreFact {
        current_period: fact.current_period,
        persisted_round: u64::from(pbft.manager_field(PBFT_MGR_FIELD_ROUND)?.unwrap_or(1)),
        persisted_step: u64::from(pbft.manager_field(PBFT_MGR_FIELD_STEP)?.unwrap_or(1)),
        cacti_active_at_chain_size: fact.cacti_active_at_chain_size,
        rounds_count_dynamic_lambda: storage.metadata().rounds_count_dynamic_lambda()?,
        persisted_dynamic_lambda_ms: pbft.manager_field(PBFT_MGR_FIELD_LAMBDA)?.unwrap_or(1),
        genesis_lambda_ms: fact.genesis_lambda_ms,
        cacti_lambda_max_ms: fact.cacti_lambda_max_ms,
        cacti_lambda_default_ms: fact.cacti_lambda_default_ms,
        executed_pbft_block: pbft
            .manager_status(PBFT_MGR_STATUS_EXECUTED_BLOCK)?
            .unwrap_or(false),
        already_next_voted_value: pbft
            .manager_status(PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE)?
            .unwrap_or(false),
        already_next_voted_null: pbft
            .manager_status(PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH)?
            .unwrap_or(false),
    });

    if snapshot.status != PbftManagerStartupRestoreStatus::Ready {
        return Err(anyhow!(snapshot.error_code.clone()));
    }
    if snapshot.persist_normalized_step {
        pbft.write_manager_field(
            PBFT_MGR_FIELD_STEP,
            u32::try_from(snapshot.step)
                .map_err(|_| anyhow!("PBFT_MANAGER_STARTUP_NORMALIZED_STEP_OVERFLOW"))?,
        )?;
        snapshot.persist_normalized_step = false;
    }

    Ok(PbftManagerRuntime::new(snapshot))
}

/// Persists one PBFT manager cursor field through Rust-owned storage.
///
/// Inputs:
/// - `storage`: shared Rust storage handle owned by the PBFT manager runtime.
/// - `field`: stable PBFT manager field id for round or step.
/// - `value`: absolute cursor value to persist.
///
/// Outputs:
/// - Writes the field to `pbft_mgr_round_step` and returns success after the
///   durable write completes.
///
/// Invariants and edge behavior:
/// - This is intentionally not a generic manager-field bridge. Dynamic lambda
///   is written by the finalization/dynamic-lambda storage paths that own that
///   state transition.
/// - Unsupported fields return an error without writing storage.
pub fn apply_pbft_manager_cursor_field_storage(
    storage: &Storage,
    field: u8,
    value: u32,
) -> Result<()> {
    match field {
        PBFT_MGR_FIELD_ROUND | PBFT_MGR_FIELD_STEP => {
            storage.pbft().write_manager_field(field, value)
        }
        _ => Err(anyhow!(
            "unsupported PBFT manager cursor field for runtime storage write: {field}"
        )),
    }
}

/// Persists the PBFT manager's latest cert-voted block through Rust storage.
///
/// Inputs:
/// - `storage`: shared Rust storage handle owned by the PBFT manager runtime.
/// - `round`: PBFT round that produced the cert vote.
/// - `block_rlp`: canonical signed PBFT block RLP payload.
///
/// Outputs:
/// - Stores the legacy `[round, block_rlp]` row in
///   `cert_voted_block_in_round` and returns after the write completes.
///
/// Invariants and edge behavior:
/// - Empty block payloads are rejected before storage writes because restart
///   recovery cannot materialize a PBFT block from an empty sidecar.
/// - The row is overwritten on each successful cert vote, matching legacy
///   RocksDB put semantics.
pub fn save_cert_voted_block_in_round_storage(
    storage: &Storage,
    round: u64,
    block_rlp: &[u8],
) -> Result<()> {
    if block_rlp.is_empty() {
        return Err(anyhow!("PBFT_MANAGER_CERT_VOTED_BLOCK_EMPTY_PAYLOAD"));
    }
    storage
        .pbft()
        .write_cert_voted_block_in_round(round, block_rlp)
}

/// Loads one finalized period needed by the PBFT manager startup replay from
/// native Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `period`: finalized PBFT period to replay.
/// - `load_period_lambda`: whether the caller needs the closest persisted
///   dynamic-lambda value for Cacti reward replay.
///
/// Outputs:
/// - `found = false` if the period data row is missing.
/// - Otherwise the canonical period data RLP, finalized DAG hashes decoded from
///   the period data, and optional period lambda.
///
/// Invariants and edge behavior:
/// - The helper only reads storage and derives hashes from canonical stored
///   bytes; C++ may still materialize temporary `PeriodData` objects from the
///   returned RLP while the live replay boundary remains transitional.
/// - Malformed period data is reported as an error so startup does not silently
///   route through legacy `DbStorage` decoding.
pub fn load_pbft_manager_startup_replay_period(
    storage: &Storage,
    period: u64,
    load_period_lambda: bool,
) -> Result<PbftManagerStartupReplayPeriod> {
    let period_data_rlp = storage.period().data_raw(period)?;
    if period_data_rlp.is_empty() {
        return Ok(PbftManagerStartupReplayPeriod {
            found: false,
            period_data_rlp,
            finalized_dag_hashes: Vec::new(),
            period_lambda: None,
        });
    }

    let finalized_dag_hashes = finalized_dag_hashes_from_period_data(&period_data_rlp)
        .with_context(|| format!("PBFT_MANAGER_STARTUP_PERIOD_DATA_DAG_HASHES_INVALID:{period}"))?;
    let period_lambda = if load_period_lambda {
        storage.metadata().period_lambda(period, true)?
    } else {
        None
    };

    Ok(PbftManagerStartupReplayPeriod {
        found: true,
        period_data_rlp,
        finalized_dag_hashes,
        period_lambda,
    })
}

fn finalized_dag_hashes_from_period_data(period_data_rlp: &[u8]) -> Result<Vec<H256>> {
    let period_data = rlp::Rlp::new(period_data_rlp);
    let dag_blocks_data = period_data.at(2)?;
    let bundle = FinalizedDagBlockBundleRlp::new(dag_blocks_data.as_raw());
    let mut hashes = Vec::with_capacity(dag_blocks_data.at(2)?.item_count()?);
    for position in 0..dag_blocks_data.at(2)?.item_count()? {
        hashes.push(keccak256(&bundle.canonical_block_rlp(position)?));
    }
    Ok(hashes)
}

/// Persists the delayed executed-block manager-status reset.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
///
/// Outputs:
/// - Writes `PbftMgrStatus::ExecutedBlock = false` through `rustaxa-storage`.
///
/// Invariants and edge behavior:
/// - This owns only the durable status row. Callers must update live/runtime
///   mirrors only after this function returns success.
/// - The post-`waitForPeriodFinalization()` ordering remains owned by the
///   PBFT manager runtime/shim boundary until that executor moves to Rust.
pub fn apply_executed_block_reset_storage(storage: &Storage) -> Result<()> {
    storage
        .pbft()
        .write_manager_status(PBFT_MGR_STATUS_EXECUTED_BLOCK, false)
        .context("PBFT_MANAGER_EXECUTED_BLOCK_RESET_WRITE")
}

/// Persists a successful next-vote manager status through Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `status`: PBFT manager status field. Only `NextVotedSoftValue` and
///   `NextVotedNullBlockHash` are accepted.
///
/// Outputs:
/// - Writes the accepted status row as `true`.
///
/// Invariants and edge behavior:
/// - This helper owns only the durable status row. Vote generation, vote
///   gossip, and live C++ mirror flags remain executor-side boundaries until
///   the state-action executor moves to Rust.
/// - Any status outside the next-voted family is rejected so this cannot become
///   a generic PBFT manager status bridge.
pub fn apply_next_voted_status_storage(storage: &Storage, status: u8) -> Result<()> {
    match status {
        PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE | PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH => {
            storage
                .pbft()
                .write_manager_status(status, true)
                .context("PBFT_MANAGER_NEXT_VOTED_STATUS_WRITE")
        }
        _ => Err(anyhow!("PBFT_MANAGER_NEXT_VOTED_STATUS_UNSUPPORTED")),
    }
}

fn transition_storage_applied(applied_writes: u64) -> PbftManagerTransitionStorageResult {
    PbftManagerTransitionStorageResult {
        status: PbftManagerTransitionStorageStatus::Applied,
        applied_writes,
        error_code: String::new(),
    }
}

fn transition_storage_rejected(error_code: &str) -> PbftManagerTransitionStorageResult {
    PbftManagerTransitionStorageResult {
        status: PbftManagerTransitionStorageStatus::Rejected,
        applied_writes: 0,
        error_code: error_code.to_string(),
    }
}

fn to_manager_u32(
    value: u64,
    error_code: &str,
) -> std::result::Result<u32, PbftManagerTransitionStorageResult> {
    u32::try_from(value).map_err(|_| transition_storage_rejected(error_code))
}

fn append_transition_storage_to_batch(
    storage: &Storage,
    batch: &mut StorageWriteBatch,
    plan: &PbftManagerTransitionPlan,
) -> std::result::Result<u64, PbftManagerTransitionStorageResult> {
    let mut applied_writes = 0;
    let pbft = storage.pbft();

    if plan.persist_round {
        let round = to_manager_u32(
            plan.new_round,
            "PBFT_MANAGER_TRANSITION_STORAGE_ROUND_OVERFLOW",
        )?;
        if pbft
            .write_manager_field_in_batch(batch, PBFT_MGR_FIELD_ROUND, round)
            .is_err()
        {
            return Err(transition_storage_rejected(
                "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
            ));
        }
        applied_writes += 1;
    }

    if plan.persist_step {
        let step = to_manager_u32(
            plan.new_step,
            "PBFT_MANAGER_TRANSITION_STORAGE_STEP_OVERFLOW",
        )?;
        if pbft
            .write_manager_field_in_batch(batch, PBFT_MGR_FIELD_STEP, step)
            .is_err()
        {
            return Err(transition_storage_rejected(
                "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
            ));
        }
        applied_writes += 1;
    }

    if plan.reset_next_voted_statuses {
        if pbft
            .write_manager_status_in_batch(batch, PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH, false)
            .and_then(|_| {
                pbft.write_manager_status_in_batch(
                    batch,
                    PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE,
                    false,
                )
            })
            .is_err()
        {
            return Err(transition_storage_rejected(
                "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
            ));
        }
        applied_writes += 2;
    }

    if plan.remove_cert_voted_block {
        if pbft
            .remove_cert_voted_block_in_round_in_batch(batch)
            .is_err()
        {
            return Err(transition_storage_rejected(
                "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
            ));
        }
        applied_writes += 1;
    }

    Ok(applied_writes)
}

/// Applies PBFT manager transition persistence in one Rust-owned storage batch.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `plan`: accepted transition plan from Rust PBFT manager planning/runtime.
/// - `own_vote_hashes`: latest-round own-vote keys to delete when
///   `plan.clear_own_votes` is set.
/// - `sync`: RocksDB write-sync setting for the committed batch.
///
/// Outputs:
/// - A storage result with stable status, applied write count, and rejection
///   code.
///
/// Invariants and edge behavior:
/// - This owns the full durable commit for manager cursor/status transitions
///   and latest-round own-vote cleanup.
/// - Callers must advance Rust runtime state and C++ mirrors only after an
///   `Applied` result.
/// - Executed-block reset remains outside this batch to preserve the
///   post-`waitForPeriodFinalization()` ordering until the executor moves to
///   Rust.
pub fn apply_pbft_manager_transition_storage(
    storage: &Storage,
    plan: &PbftManagerTransitionPlan,
    own_vote_hashes: &[H256],
    sync: bool,
) -> Result<PbftManagerTransitionStorageResult> {
    if plan.status != PbftManagerTransitionStatus::Ready {
        return Ok(transition_storage_rejected(
            "PBFT_MANAGER_TRANSITION_STORAGE_PLAN_NOT_READY",
        ));
    }
    if !plan.clear_own_votes && !own_vote_hashes.is_empty() {
        return Ok(transition_storage_rejected(
            "PBFT_MANAGER_TRANSITION_STORAGE_UNEXPECTED_OWN_VOTE_HASHES",
        ));
    }

    let mut batch = storage.create_write_batch();
    let mut applied_writes = match append_transition_storage_to_batch(storage, &mut batch, plan) {
        Ok(applied_writes) => applied_writes,
        Err(result) => return Ok(result),
    };

    if plan.clear_own_votes {
        for hash in own_vote_hashes {
            if storage
                .pbft()
                .remove_own_verified_vote_in_batch(&mut batch, *hash)
                .is_err()
            {
                return Ok(transition_storage_rejected(
                    "PBFT_MANAGER_TRANSITION_STORAGE_WRITE_FAILURE",
                ));
            }
        }
        applied_writes += own_vote_hashes.len() as u64;
    }

    if storage.commit_write_batch_with_sync(batch, sync).is_err() {
        return Ok(transition_storage_rejected(
            "PBFT_MANAGER_TRANSITION_STORAGE_COMMIT_FAILURE",
        ));
    }

    Ok(transition_storage_applied(applied_writes))
}

/// C++-originated facts for deciding whether PBFT can advance to a new round.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerAdvanceRoundFact {
    /// Current PBFT period.
    pub period: u64,
    /// Current PBFT round.
    pub current_round: u64,
    /// Whether C++/VoteManager found a candidate new round.
    pub has_new_round: bool,
    /// Candidate new round when present.
    pub new_round: u64,
}

/// Side-effect-free plan for PBFT round advancement.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerAdvanceRoundPlan {
    /// Planning status.
    pub status: PbftManagerTransitionStatus,
    /// Whether C++ should apply a reset transition to `target_round`.
    pub should_advance: bool,
    /// Planned target round when `should_advance` is true.
    pub target_round: u64,
    /// Stable error detail for rejected facts.
    pub error_code: String,
}

fn ready_state_action_plan(
    primary_intent: PbftManagerStateActionIntent,
    primary_hash: [u8; 32],
    secondary_intent: PbftManagerStateActionIntent,
    secondary_hash: [u8; 32],
    go_finish_state: bool,
    loop_back_finish_state: bool,
) -> PbftManagerStateActionPlan {
    PbftManagerStateActionPlan {
        status: PbftManagerStateActionStatus::Ready,
        primary_intent,
        primary_hash,
        secondary_intent,
        secondary_hash,
        go_finish_state,
        loop_back_finish_state,
        error_code: String::new(),
    }
}

fn reject_state_action_plan(
    status: PbftManagerStateActionStatus,
    error_code: &str,
) -> PbftManagerStateActionPlan {
    PbftManagerStateActionPlan {
        status,
        primary_intent: PbftManagerStateActionIntent::Noop,
        primary_hash: [0; 32],
        secondary_intent: PbftManagerStateActionIntent::Noop,
        secondary_hash: [0; 32],
        go_finish_state: false,
        loop_back_finish_state: false,
        error_code: error_code.to_string(),
    }
}

fn reject_transition_plan(
    status: PbftManagerTransitionStatus,
    kind: PbftManagerTransitionKind,
    error_code: &str,
) -> PbftManagerTransitionPlan {
    PbftManagerTransitionPlan {
        status,
        kind,
        new_state: PbftManagerRuntimeStateCode::Unknown,
        new_round: 0,
        new_step: 0,
        current_round_lambda_ms: 0,
        next_step_time_ms: 0,
        persist_round: false,
        persist_step: false,
        reset_next_voted_statuses: false,
        remove_cert_voted_block: false,
        clear_own_votes: false,
        clear_broadcasted_votes: false,
        reset_broadcast_counters: false,
        reset_executed_block_status: false,
        set_vote_manager_period_round: false,
        reset_current_round_start: false,
        reset_second_finish_start: false,
        print_cert_step_info: false,
        print_second_finish_step_info: false,
        error_code: error_code.to_string(),
    }
}

fn transition_base_plan(
    fact: &PbftManagerTransitionFact,
    new_state: PbftManagerRuntimeStateCode,
    new_round: u64,
    new_step: u64,
    current_round_lambda_ms: u64,
    next_step_time_ms: u64,
) -> PbftManagerTransitionPlan {
    PbftManagerTransitionPlan {
        status: PbftManagerTransitionStatus::Ready,
        kind: fact.kind,
        new_state,
        new_round,
        new_step,
        current_round_lambda_ms,
        next_step_time_ms,
        persist_round: false,
        persist_step: true,
        reset_next_voted_statuses: false,
        remove_cert_voted_block: false,
        clear_own_votes: false,
        clear_broadcasted_votes: false,
        reset_broadcast_counters: false,
        reset_executed_block_status: false,
        set_vote_manager_period_round: false,
        reset_current_round_start: false,
        reset_second_finish_start: false,
        print_cert_step_info: false,
        print_second_finish_step_info: false,
        error_code: String::new(),
    }
}

fn planned_lambda_for_step(fact: &PbftManagerTransitionFact, new_step: u64) -> u64 {
    if new_step >= fact.max_steps && new_step % 2 == 1 {
        let mut lambda = if new_step == fact.max_steps {
            fact.default_lambda_ms
        } else {
            fact.current_round_lambda_ms
        };
        let catch_up_delay = fact.max_steps.saturating_sub(4);
        if fact.network_next_voting_step > new_step
            && fact.network_next_voting_step - new_step >= catch_up_delay
        {
            fact.default_lambda_ms
        } else if lambda < fact.max_exponential_lambda_ms {
            lambda = lambda.saturating_mul(2).min(fact.max_exponential_lambda_ms);
            lambda
        } else {
            lambda
        }
    } else {
        fact.current_round_lambda_ms
    }
}

fn validate_transition_fact(fact: &PbftManagerTransitionFact) -> Option<&'static str> {
    if fact.kind == PbftManagerTransitionKind::Unknown {
        return Some("PBFT_MANAGER_TRANSITION_UNKNOWN_KIND");
    }
    if fact.period == 0 || fact.round == 0 || fact.step == 0 {
        return Some("PBFT_MANAGER_TRANSITION_INVALID_CURSOR");
    }
    if fact.current_round_lambda_ms == 0
        || fact.default_lambda_ms == 0
        || fact.max_exponential_lambda_ms == 0
        || fact.max_steps == 0
    {
        return Some("PBFT_MANAGER_TRANSITION_INVALID_TIMING_FACTS");
    }
    if fact.kind == PbftManagerTransitionKind::ResetConsensus && fact.target_round == 0 {
        return Some("PBFT_MANAGER_TRANSITION_INVALID_TARGET_ROUND");
    }
    None
}

/// Plans one PBFT manager cursor/status transition from explicit protocol facts.
///
/// The plan is side-effect-free. It owns the deterministic state/round/step,
/// lambda, next-step timing, and manager-status reset decisions. C++ consumes
/// the plan as an executor by persisting fields, updating live compatibility
/// state, clearing sidecars, and setting timestamps.
pub fn plan_pbft_manager_transition(fact: PbftManagerTransitionFact) -> PbftManagerTransitionPlan {
    if let Some(error) = validate_transition_fact(&fact) {
        let status = if fact.kind == PbftManagerTransitionKind::Unknown {
            PbftManagerTransitionStatus::InvalidKind
        } else {
            PbftManagerTransitionStatus::InvalidFact
        };
        return reject_transition_plan(status, fact.kind, error);
    }

    match fact.kind {
        PbftManagerTransitionKind::ResetConsensus => {
            let lambda = if fact.cacti_hardfork {
                fact.target_round_lambda_ms
            } else {
                fact.default_lambda_ms
            };
            if lambda == 0 {
                return reject_transition_plan(
                    PbftManagerTransitionStatus::InvalidFact,
                    fact.kind,
                    "PBFT_MANAGER_TRANSITION_INVALID_RESET_LAMBDA",
                );
            }
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::ValueProposal,
                fact.target_round,
                1,
                lambda,
                fact.next_step_time_ms,
            );
            plan.persist_round = true;
            plan.reset_next_voted_statuses = true;
            plan.remove_cert_voted_block = fact.has_cert_voted_block;
            plan.clear_own_votes = true;
            plan.clear_broadcasted_votes = true;
            plan.reset_broadcast_counters = true;
            plan.reset_executed_block_status = fact.executed_pbft_block;
            plan.set_vote_manager_period_round = true;
            plan.reset_current_round_start = true;
            plan
        }
        PbftManagerTransitionKind::ToFilter => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Filter,
                fact.round,
                new_step,
                lambda,
                lambda.saturating_mul(2),
            )
        }
        PbftManagerTransitionKind::ToCertify => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Certify,
                fact.round,
                new_step,
                lambda,
                lambda.saturating_mul(2),
            );
            plan.print_cert_step_info = true;
            plan
        }
        PbftManagerTransitionKind::ToFinish => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Finish,
                fact.round,
                new_step,
                lambda,
                fact.deadline_ms,
            )
        }
        PbftManagerTransitionKind::ToFinishPolling => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::FinishPolling,
                fact.round,
                new_step,
                lambda,
                fact.next_step_time_ms
                    .saturating_add(fact.polling_interval_ms),
            );
            plan.reset_next_voted_statuses = true;
            plan.reset_second_finish_start = true;
            plan.print_second_finish_step_info = true;
            plan
        }
        PbftManagerTransitionKind::LoopBackFinish => {
            let new_step = fact.step.saturating_add(1);
            let lambda = planned_lambda_for_step(&fact, new_step);
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Finish,
                fact.round,
                new_step,
                lambda,
                fact.next_step_time_ms
                    .saturating_add(fact.polling_interval_ms),
            );
            plan.reset_next_voted_statuses = true;
            plan
        }
        PbftManagerTransitionKind::DelayCertifyPoll => {
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::Certify,
                fact.round,
                fact.step,
                fact.current_round_lambda_ms,
                fact.next_step_time_ms
                    .saturating_add(fact.polling_interval_ms),
            );
            plan.persist_step = false;
            plan
        }
        PbftManagerTransitionKind::DelayFinishPoll => {
            let mut plan = transition_base_plan(
                &fact,
                PbftManagerRuntimeStateCode::FinishPolling,
                fact.round,
                fact.step,
                fact.current_round_lambda_ms,
                fact.next_step_time_ms
                    .saturating_add(fact.polling_interval_ms),
            );
            plan.persist_step = false;
            plan
        }
        PbftManagerTransitionKind::Unknown => unreachable!("unknown transition rejected above"),
    }
}

/// Plans whether a PBFT manager round-advance candidate should reset consensus.
///
/// C++ still sources the candidate from the VoteManager/verified-votes runtime,
/// but Rust validates the protocol condition that advancement requires a
/// strictly greater round before C++ applies the reset transition.
pub fn plan_pbft_manager_advance_round(
    fact: PbftManagerAdvanceRoundFact,
) -> PbftManagerAdvanceRoundPlan {
    if fact.period == 0 || fact.current_round == 0 {
        return PbftManagerAdvanceRoundPlan {
            status: PbftManagerTransitionStatus::InvalidFact,
            should_advance: false,
            target_round: 0,
            error_code: "PBFT_MANAGER_ADVANCE_ROUND_INVALID_CURSOR".to_string(),
        };
    }
    if !fact.has_new_round {
        return PbftManagerAdvanceRoundPlan {
            status: PbftManagerTransitionStatus::Ready,
            should_advance: false,
            target_round: 0,
            error_code: String::new(),
        };
    }
    if fact.new_round <= fact.current_round {
        return PbftManagerAdvanceRoundPlan {
            status: PbftManagerTransitionStatus::InvalidFact,
            should_advance: false,
            target_round: 0,
            error_code: "PBFT_MANAGER_ADVANCE_ROUND_NON_INCREASING_ROUND".to_string(),
        };
    }
    PbftManagerAdvanceRoundPlan {
        status: PbftManagerTransitionStatus::Ready,
        should_advance: true,
        target_round: fact.new_round,
        error_code: String::new(),
    }
}

/// Plans one PBFT manager state action from explicit protocol facts.
///
/// The plan is side-effect-free. It deliberately does not validate or
/// materialize PBFT blocks, generate votes, write storage, sleep, gossip, or
/// execute FinalChain/EVM logic. Those remain executor responsibilities around
/// the Rust-owned protocol branch decision.
pub fn plan_pbft_manager_state_action(
    fact: PbftManagerStateActionFact,
) -> PbftManagerStateActionPlan {
    if fact.state == PbftManagerRuntimeStateCode::Unknown {
        return reject_state_action_plan(
            PbftManagerStateActionStatus::InvalidState,
            "PBFT_MANAGER_STATE_ACTION_UNKNOWN_STATE",
        );
    }
    if fact.period == 0 || fact.round == 0 || fact.step == 0 {
        return reject_state_action_plan(
            PbftManagerStateActionStatus::InvalidFact,
            "PBFT_MANAGER_STATE_ACTION_INVALID_CURSOR",
        );
    }

    match fact.state {
        PbftManagerRuntimeStateCode::ValueProposal => plan_value_proposal_state_action(&fact),
        PbftManagerRuntimeStateCode::Filter => plan_filter_state_action(&fact),
        PbftManagerRuntimeStateCode::Certify => plan_certify_state_action(&fact),
        PbftManagerRuntimeStateCode::Finish => plan_first_finish_state_action(&fact),
        PbftManagerRuntimeStateCode::FinishPolling => plan_second_finish_state_action(&fact),
        PbftManagerRuntimeStateCode::Unknown => unreachable!("unknown state rejected above"),
    }
}

fn previous_round_starts_from_null(fact: &PbftManagerStateActionFact) -> bool {
    fact.round == 1 || fact.has_previous_round_next_null
}

fn plan_value_proposal_state_action(
    fact: &PbftManagerStateActionFact,
) -> PbftManagerStateActionPlan {
    if previous_round_starts_from_null(fact) {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::ProposeNewBlock,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.has_previous_round_next_value {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::ReproposePreviousRoundNextValue,
            fact.previous_round_next_value_hash,
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    reject_state_action_plan(
        PbftManagerStateActionStatus::InvalidFact,
        "PBFT_MANAGER_VALUE_PROPOSAL_MISSING_PREVIOUS_ROUND_STARTING_VALUE",
    )
}

fn plan_filter_state_action(fact: &PbftManagerStateActionFact) -> PbftManagerStateActionPlan {
    if previous_round_starts_from_null(fact) {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::IdentifyLeaderAndSoftVote,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.has_previous_round_next_value {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::SoftVotePreviousRoundNextValue,
            fact.previous_round_next_value_hash,
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    ready_state_action_plan(
        PbftManagerStateActionIntent::Noop,
        [0; 32],
        PbftManagerStateActionIntent::Noop,
        [0; 32],
        false,
        false,
    )
}

fn plan_certify_state_action(fact: &PbftManagerStateActionFact) -> PbftManagerStateActionPlan {
    let finish_deadline_ms = fact.deadline_ms.saturating_sub(fact.polling_interval_ms);
    let go_finish_state = fact.elapsed_round_ms > finish_deadline_ms;
    if go_finish_state {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::GoFinish,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            true,
            false,
        );
    }

    if fact.elapsed_round_ms < fact.current_round_lambda_ms.saturating_mul(2) {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.has_cert_voted_block || !fact.has_current_round_soft_value {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    ready_state_action_plan(
        PbftManagerStateActionIntent::CertVoteCurrentSoftValue,
        fact.current_round_soft_value_hash,
        PbftManagerStateActionIntent::Noop,
        [0; 32],
        false,
        false,
    )
}

fn plan_first_finish_state_action(fact: &PbftManagerStateActionFact) -> PbftManagerStateActionPlan {
    if fact.has_cert_voted_block {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::NextVoteCertVotedBlock,
            fact.cert_voted_block_hash,
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.round >= 2 && fact.has_previous_round_next_null {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::NextVoteNullBlock,
            [0; 32],
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    if fact.has_previous_round_next_value {
        return ready_state_action_plan(
            PbftManagerStateActionIntent::NextVotePreviousRoundValue,
            fact.previous_round_next_value_hash,
            PbftManagerStateActionIntent::Noop,
            [0; 32],
            false,
            false,
        );
    }

    ready_state_action_plan(
        PbftManagerStateActionIntent::NextVoteNullBlock,
        [0; 32],
        PbftManagerStateActionIntent::Noop,
        [0; 32],
        false,
        false,
    )
}

fn plan_second_finish_state_action(
    fact: &PbftManagerStateActionFact,
) -> PbftManagerStateActionPlan {
    let primary = if !fact.already_next_voted_value && fact.has_current_round_soft_value {
        PbftManagerStateActionIntent::NextVoteCurrentSoftValue
    } else {
        PbftManagerStateActionIntent::Noop
    };
    let primary_hash = if primary == PbftManagerStateActionIntent::NextVoteCurrentSoftValue {
        fact.current_round_soft_value_hash
    } else {
        [0; 32]
    };

    let secondary = if !fact.has_cert_voted_block
        && !fact.already_next_voted_null
        && fact.round >= 2
        && fact.has_previous_round_next_null
    {
        PbftManagerStateActionIntent::NextVoteNullBlock
    } else {
        PbftManagerStateActionIntent::Noop
    };

    let loop_back_finish_state = fact.elapsed_round_ms
        > fact
            .current_round_lambda_ms
            .saturating_sub(fact.polling_interval_ms)
            .saturating_mul(2);

    ready_state_action_plan(
        primary,
        primary_hash,
        secondary,
        [0; 32],
        false,
        loop_back_finish_state,
    )
}

/// One C++ action report for the Rust-owned PBFT manager runtime cursor.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntimeActionReport {
    /// Cursor returned by the previous session step.
    pub cursor: u32,
    /// Action that C++ executed.
    pub action: PbftManagerRuntimeAction,
    /// Whether the action call itself succeeded.
    pub success: bool,
    /// Stable action result code.
    pub result: PbftManagerRuntimeActionResultCode,
    /// `go_finish_state_` observed after `RunCertify`.
    pub go_finish_state: bool,
    /// `loop_back_finish_state_` observed after `RunSecondFinish`.
    pub loop_back_finish_state: bool,
    /// Current eligible-wallet state after the reported action.
    pub has_eligible_wallet: bool,
    /// Whether C++/VoteManager found a candidate new round for
    /// `TryAdvanceRound`.
    pub has_new_round: bool,
    /// Candidate new round reported for `TryAdvanceRound`, when present.
    pub new_round: u64,
    /// Optional error detail from the C++ executor.
    pub error_code: String,
}

/// One Rust-owned session step for C++ execution.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntimeSessionStep {
    /// Session status.
    pub status: PbftManagerRuntimeStatus,
    /// Cursor for the returned action.
    pub cursor: u32,
    /// Action to execute, if any.
    pub action: Option<PbftManagerRuntimeAction>,
    /// Whether `action` is valid.
    pub has_action: bool,
    /// Whether the session completed all actions.
    pub complete: bool,
    /// Whether C++ should restart the daemon loop immediately.
    pub restart_loop: bool,
    /// Whether this step carries a target round for a reset-consensus effect.
    pub has_target_round: bool,
    /// Target round for `ResetConsensus` when `has_target_round` is true.
    pub target_round: u64,
    /// Caller-local tick id.
    pub tick_id: u64,
    /// Stable error detail.
    pub error_code: String,
}

/// Stateful Rust cursor for one PBFT manager daemon tick.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftManagerRuntimeSession {
    /// Immutable tick facts.
    pub fact: PbftManagerRuntimeTickFact,
    /// Current session status.
    pub status: PbftManagerRuntimeStatus,
    /// Pending actions that have not yet been handed to C++.
    pub pending: VecDeque<PbftManagerRuntimeAction>,
    /// Cursor of the next action.
    pub cursor: u32,
    /// Completed restart-loop signal.
    pub restart_loop: bool,
    /// Target round attached to a pending `ResetConsensus` action.
    pub reset_target_round: Option<u64>,
    /// Stable error detail.
    pub error_code: String,
}

fn reject_session(fact: PbftManagerRuntimeTickFact, error_code: &str) -> PbftManagerRuntimeSession {
    PbftManagerRuntimeSession {
        fact,
        status: PbftManagerRuntimeStatus::RejectedTick,
        pending: VecDeque::new(),
        cursor: 0,
        restart_loop: false,
        reset_target_round: None,
        error_code: error_code.to_string(),
    }
}

fn append_state_script(
    actions: &mut VecDeque<PbftManagerRuntimeAction>,
    state: PbftManagerRuntimeStateCode,
) {
    match state {
        PbftManagerRuntimeStateCode::ValueProposal => {
            actions.push_back(PbftManagerRuntimeAction::RunValueProposal);
            actions.push_back(PbftManagerRuntimeAction::TransitionToFilter);
            actions.push_back(PbftManagerRuntimeAction::SleepUntilNextStep);
        }
        PbftManagerRuntimeStateCode::Filter => {
            actions.push_back(PbftManagerRuntimeAction::RunFilter);
            actions.push_back(PbftManagerRuntimeAction::TransitionToCertify);
            actions.push_back(PbftManagerRuntimeAction::SleepUntilNextStep);
        }
        PbftManagerRuntimeStateCode::Certify => {
            actions.push_back(PbftManagerRuntimeAction::RunCertify);
        }
        PbftManagerRuntimeStateCode::Finish => {
            actions.push_back(PbftManagerRuntimeAction::RunFirstFinish);
            actions.push_back(PbftManagerRuntimeAction::TransitionToFinishPolling);
            actions.push_back(PbftManagerRuntimeAction::SleepUntilNextStep);
        }
        PbftManagerRuntimeStateCode::FinishPolling => {
            actions.push_back(PbftManagerRuntimeAction::RunSecondFinish);
        }
        PbftManagerRuntimeStateCode::Unknown => {}
    }
}

/// Creates a Rust-owned PBFT manager runtime session for one daemon tick.
pub fn create_pbft_manager_runtime_session(
    fact: PbftManagerRuntimeTickFact,
) -> PbftManagerRuntimeSession {
    if fact.state == PbftManagerRuntimeStateCode::Unknown {
        return reject_session(fact, "PBFT_MANAGER_RUNTIME_UNKNOWN_STATE");
    }

    if fact.period == 0 || fact.round == 0 || fact.step == 0 {
        return reject_session(fact, "PBFT_MANAGER_RUNTIME_INVALID_CURSOR");
    }

    let mut pending = VecDeque::new();
    pending.push_back(PbftManagerRuntimeAction::ProcessSyncedPbftBlocks);
    if fact.network_available && !fact.network_pbft_syncing {
        pending.push_back(PbftManagerRuntimeAction::MaybeBroadcastVotes);
        pending.push_back(PbftManagerRuntimeAction::TryPushCertVotesBlock);
    }
    pending.push_back(PbftManagerRuntimeAction::TryAdvanceRound);

    PbftManagerRuntimeSession {
        fact,
        status: PbftManagerRuntimeStatus::Active,
        pending,
        cursor: 0,
        restart_loop: false,
        reset_target_round: None,
        error_code: String::new(),
    }
}

/// Returns the next action for a PBFT manager runtime session.
pub fn next_pbft_manager_runtime_action(
    session: &PbftManagerRuntimeSession,
) -> PbftManagerRuntimeSessionStep {
    if session.status != PbftManagerRuntimeStatus::Active {
        return PbftManagerRuntimeSessionStep {
            status: session.status,
            cursor: session.cursor,
            action: None,
            has_action: false,
            complete: session.status == PbftManagerRuntimeStatus::Complete,
            restart_loop: session.restart_loop,
            has_target_round: false,
            target_round: 0,
            tick_id: session.fact.tick_id,
            error_code: session.error_code.clone(),
        };
    }

    match session.pending.front().copied() {
        Some(action) => {
            let target_round = if action == PbftManagerRuntimeAction::ResetConsensus {
                session.reset_target_round.unwrap_or(0)
            } else {
                0
            };
            PbftManagerRuntimeSessionStep {
                status: PbftManagerRuntimeStatus::Active,
                cursor: session.cursor,
                action: Some(action),
                has_action: true,
                complete: false,
                restart_loop: false,
                has_target_round: action == PbftManagerRuntimeAction::ResetConsensus,
                target_round,
                tick_id: session.fact.tick_id,
                error_code: String::new(),
            }
        }
        None => PbftManagerRuntimeSessionStep {
            status: PbftManagerRuntimeStatus::Complete,
            cursor: session.cursor,
            action: None,
            has_action: false,
            complete: true,
            restart_loop: session.restart_loop,
            has_target_round: false,
            target_round: 0,
            tick_id: session.fact.tick_id,
            error_code: String::new(),
        },
    }
}

fn fail_session(
    mut session: PbftManagerRuntimeSession,
    status: PbftManagerRuntimeStatus,
    error_code: String,
) -> PbftManagerRuntimeSession {
    session.status = status;
    session.pending.clear();
    session.reset_target_round = None;
    session.error_code = error_code;
    session
}

fn report_error(report: &PbftManagerRuntimeActionReport, fallback: &str) -> String {
    if report.error_code.is_empty() {
        fallback.to_string()
    } else {
        report.error_code.clone()
    }
}

fn valid_action_result(
    action: PbftManagerRuntimeAction,
    result: PbftManagerRuntimeActionResultCode,
) -> bool {
    match action {
        PbftManagerRuntimeAction::TryPushCertVotesBlock => matches!(
            result,
            PbftManagerRuntimeActionResultCode::NoProgressContinue
                | PbftManagerRuntimeActionResultCode::ProgressRestartLoop
        ),
        PbftManagerRuntimeAction::TryAdvanceRound => {
            result == PbftManagerRuntimeActionResultCode::NoProgressContinue
        }
        PbftManagerRuntimeAction::ResetConsensus => {
            result == PbftManagerRuntimeActionResultCode::TransitionApplied
        }
        PbftManagerRuntimeAction::TransitionToFilter
        | PbftManagerRuntimeAction::TransitionToCertify
        | PbftManagerRuntimeAction::TransitionToFinish
        | PbftManagerRuntimeAction::TransitionToFinishPolling
        | PbftManagerRuntimeAction::LoopBackFinish => {
            result == PbftManagerRuntimeActionResultCode::TransitionApplied
        }
        PbftManagerRuntimeAction::SleepIneligiblePollingInterval
        | PbftManagerRuntimeAction::DelayCertifyPoll
        | PbftManagerRuntimeAction::DelayFinishPoll
        | PbftManagerRuntimeAction::SleepUntilNextStep => {
            result == PbftManagerRuntimeActionResultCode::SleepApplied
        }
        PbftManagerRuntimeAction::ProcessSyncedPbftBlocks
        | PbftManagerRuntimeAction::MaybeBroadcastVotes
        | PbftManagerRuntimeAction::RunValueProposal
        | PbftManagerRuntimeAction::RunFilter
        | PbftManagerRuntimeAction::RunCertify
        | PbftManagerRuntimeAction::RunFirstFinish
        | PbftManagerRuntimeAction::RunSecondFinish => {
            result == PbftManagerRuntimeActionResultCode::StateActionDone
        }
        PbftManagerRuntimeAction::Unknown => false,
    }
}

/// Reports a C++-executed manager action and advances the Rust cursor.
pub fn report_pbft_manager_runtime_action(
    mut session: PbftManagerRuntimeSession,
    report: PbftManagerRuntimeActionReport,
) -> PbftManagerRuntimeSession {
    if session.status != PbftManagerRuntimeStatus::Active {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ContractError,
            "PBFT_MANAGER_RUNTIME_NOT_ACTIVE".to_string(),
        );
    }

    if report.cursor != session.cursor {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ActionMismatch,
            "PBFT_MANAGER_RUNTIME_CURSOR_MISMATCH".to_string(),
        );
    }

    let Some(expected_action) = session.pending.pop_front() else {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ContractError,
            "PBFT_MANAGER_RUNTIME_MISSING_ACTION".to_string(),
        );
    };

    if report.action != expected_action {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ActionMismatch,
            "PBFT_MANAGER_RUNTIME_ACTION_MISMATCH".to_string(),
        );
    }

    if !report.success || report.result == PbftManagerRuntimeActionResultCode::ExecutorError {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::ActionFailed,
            report_error(&report, "PBFT_MANAGER_RUNTIME_ACTION_FAILED"),
        );
    }

    if !valid_action_result(expected_action, report.result) {
        return fail_session(
            session,
            PbftManagerRuntimeStatus::InvalidReport,
            "PBFT_MANAGER_RUNTIME_RESULT_MISMATCH".to_string(),
        );
    }

    match expected_action {
        PbftManagerRuntimeAction::TryPushCertVotesBlock => {
            if report.result == PbftManagerRuntimeActionResultCode::ProgressRestartLoop {
                session.status = PbftManagerRuntimeStatus::Complete;
                session.pending.clear();
                session.restart_loop = true;
                session.cursor = session.cursor.saturating_add(1);
                return session;
            }
        }
        PbftManagerRuntimeAction::TryAdvanceRound => {
            let advance_plan = plan_pbft_manager_advance_round(PbftManagerAdvanceRoundFact {
                period: session.fact.period,
                current_round: session.fact.round,
                has_new_round: report.has_new_round,
                new_round: report.new_round,
            });
            if advance_plan.status != PbftManagerTransitionStatus::Ready {
                return fail_session(
                    session,
                    PbftManagerRuntimeStatus::InvalidReport,
                    advance_plan.error_code,
                );
            }
            if advance_plan.should_advance {
                session.reset_target_round = Some(advance_plan.target_round);
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::ResetConsensus);
            } else if report.has_eligible_wallet {
                append_state_script(&mut session.pending, session.fact.state);
            } else {
                session
                    .pending
                    .push_back(PbftManagerRuntimeAction::SleepIneligiblePollingInterval);
            }
        }
        PbftManagerRuntimeAction::ResetConsensus => {
            session.status = PbftManagerRuntimeStatus::Complete;
            session.pending.clear();
            session.reset_target_round = None;
            session.restart_loop = true;
            session.cursor = session.cursor.saturating_add(1);
            return session;
        }
        PbftManagerRuntimeAction::SleepIneligiblePollingInterval => {
            if report.result != PbftManagerRuntimeActionResultCode::SleepApplied {
                return fail_session(
                    session,
                    PbftManagerRuntimeStatus::InvalidReport,
                    "PBFT_MANAGER_RUNTIME_INELIGIBLE_SLEEP_REPORT_MISMATCH".to_string(),
                );
            }
            session.status = PbftManagerRuntimeStatus::Complete;
            session.pending.clear();
            session.restart_loop = true;
            session.cursor = session.cursor.saturating_add(1);
            return session;
        }
        PbftManagerRuntimeAction::RunCertify => {
            if report.go_finish_state {
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::SleepUntilNextStep);
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::TransitionToFinish);
            } else {
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::SleepUntilNextStep);
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::DelayCertifyPoll);
            }
        }
        PbftManagerRuntimeAction::RunSecondFinish => {
            if report.loop_back_finish_state {
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::SleepUntilNextStep);
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::LoopBackFinish);
            } else {
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::SleepUntilNextStep);
                session
                    .pending
                    .push_front(PbftManagerRuntimeAction::DelayFinishPoll);
            }
        }
        _ => {}
    }

    session.cursor = session.cursor.saturating_add(1);
    if session.pending.is_empty() {
        session.status = PbftManagerRuntimeStatus::Complete;
    }
    session
}

/// Marks a PBFT manager runtime session as aborted.
pub fn abort_pbft_manager_runtime_session(
    mut session: PbftManagerRuntimeSession,
) -> PbftManagerRuntimeSession {
    session.status = PbftManagerRuntimeStatus::ContractError;
    session.pending.clear();
    session.restart_loop = false;
    session.reset_target_round = None;
    session.error_code = "PBFT_MANAGER_RUNTIME_ABORTED".to_string();
    session
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_storage::{Config, Storage};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn fact(state: PbftManagerRuntimeStateCode) -> PbftManagerRuntimeTickFact {
        PbftManagerRuntimeTickFact {
            tick_id: 42,
            state,
            period: 10,
            round: 2,
            step: 3,
            network_available: true,
            network_pbft_syncing: false,
            has_eligible_wallet: true,
        }
    }

    fn report(cursor: u32, action: PbftManagerRuntimeAction) -> PbftManagerRuntimeActionReport {
        PbftManagerRuntimeActionReport {
            cursor,
            action,
            success: true,
            result: PbftManagerRuntimeActionResultCode::StateActionDone,
            go_finish_state: false,
            loop_back_finish_state: false,
            has_eligible_wallet: true,
            has_new_round: false,
            new_round: 0,
            error_code: String::new(),
        }
    }

    fn state_fact(state: PbftManagerRuntimeStateCode) -> PbftManagerStateActionFact {
        PbftManagerStateActionFact {
            state,
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
        }
    }

    fn transition_fact(kind: PbftManagerTransitionKind) -> PbftManagerTransitionFact {
        PbftManagerTransitionFact {
            kind,
            period: 10,
            round: 2,
            step: 3,
            target_round: 4,
            current_round_lambda_ms: 100,
            target_round_lambda_ms: 400,
            default_lambda_ms: 100,
            max_exponential_lambda_ms: 60_000,
            max_steps: 13,
            network_next_voting_step: 0,
            deadline_ms: 1_000,
            polling_interval_ms: 100,
            next_step_time_ms: 900,
            cacti_hardfork: true,
            has_cert_voted_block: true,
            executed_pbft_block: true,
        }
    }

    fn startup_fact(round: u64, step: u64) -> PbftManagerStartupRestoreFact {
        PbftManagerStartupRestoreFact {
            current_period: 10,
            persisted_round: round,
            persisted_step: step,
            cacti_active_at_chain_size: true,
            rounds_count_dynamic_lambda: 7,
            persisted_dynamic_lambda_ms: 1_500,
            genesis_lambda_ms: 100,
            cacti_lambda_max_ms: 1_500,
            cacti_lambda_default_ms: 500,
            executed_pbft_block: true,
            already_next_voted_value: true,
            already_next_voted_null: false,
        }
    }

    fn storage_startup_fact() -> PbftManagerStorageStartupFact {
        PbftManagerStorageStartupFact {
            current_period: 10,
            cacti_active_at_chain_size: false,
            genesis_lambda_ms: 1_000,
            cacti_lambda_max_ms: 1_500,
            cacti_lambda_default_ms: 500,
        }
    }

    fn finalized_dag_bundle_rlp() -> (Vec<u8>, Vec<H256>) {
        let mut compact_block = RlpStream::new_list(7);
        compact_block.append(&H256::from_low_u64_be(1));
        compact_block.append(&7u64);
        compact_block.append(&123u64);
        compact_block.append(&vec![0x44, 0x55]);
        compact_block.append_list(&vec![H256::from_low_u64_be(2)]);
        compact_block.append(&vec![0x66; 65]);
        compact_block.append(&99u64);

        let mut canonical_block = RlpStream::new_list(8);
        canonical_block.append(&H256::from_low_u64_be(1));
        canonical_block.append(&7u64);
        canonical_block.append(&123u64);
        canonical_block.append(&vec![0x44, 0x55]);
        canonical_block.append_list(&vec![H256::from_low_u64_be(2)]);
        let empty_transactions: Vec<H256> = Vec::new();
        canonical_block.append_list(&empty_transactions);
        canonical_block.append(&vec![0x66; 65]);
        canonical_block.append(&99u64);
        let expected_hash = keccak256(&canonical_block.out());

        let ordered_transaction_hashes = RlpStream::new_list(0);
        let mut transaction_indexes = RlpStream::new_list(1);
        transaction_indexes.begin_list(0);
        let mut compact_blocks = RlpStream::new_list(1);
        compact_blocks.append_raw(&compact_block.out(), 1);

        let mut bundle = RlpStream::new_list(3);
        bundle.append_raw(&ordered_transaction_hashes.out(), 1);
        bundle.append_raw(&transaction_indexes.out(), 1);
        bundle.append_raw(&compact_blocks.out(), 1);

        (bundle.out().to_vec(), vec![expected_hash])
    }

    fn period_data_with_dag_bundle(bundle: &[u8]) -> Vec<u8> {
        let mut period_data = RlpStream::new_list(4);
        period_data.append_empty_data();
        period_data.append_empty_data();
        period_data.append_raw(bundle, 1);
        period_data.begin_list(0);
        period_data.out().to_vec()
    }

    fn drain_actions(mut session: PbftManagerRuntimeSession) -> Vec<PbftManagerRuntimeAction> {
        let mut actions = Vec::new();
        loop {
            let step = next_pbft_manager_runtime_action(&session);
            if !step.has_action {
                break;
            }
            let action = step.action.expect("action is present");
            actions.push(action);
            let mut action_report = report(step.cursor, action);
            action_report.result = match action {
                PbftManagerRuntimeAction::TryPushCertVotesBlock
                | PbftManagerRuntimeAction::TryAdvanceRound => {
                    PbftManagerRuntimeActionResultCode::NoProgressContinue
                }
                PbftManagerRuntimeAction::TransitionToFilter
                | PbftManagerRuntimeAction::TransitionToCertify
                | PbftManagerRuntimeAction::TransitionToFinish
                | PbftManagerRuntimeAction::TransitionToFinishPolling
                | PbftManagerRuntimeAction::LoopBackFinish
                | PbftManagerRuntimeAction::ResetConsensus => {
                    PbftManagerRuntimeActionResultCode::TransitionApplied
                }
                PbftManagerRuntimeAction::SleepUntilNextStep
                | PbftManagerRuntimeAction::SleepIneligiblePollingInterval
                | PbftManagerRuntimeAction::DelayCertifyPoll
                | PbftManagerRuntimeAction::DelayFinishPoll => {
                    PbftManagerRuntimeActionResultCode::SleepApplied
                }
                _ => PbftManagerRuntimeActionResultCode::StateActionDone,
            };
            session = report_pbft_manager_runtime_action(session, action_report);
        }
        actions
    }

    #[test]
    fn value_proposal_tick_orders_prestate_state_transition_and_sleep() {
        let actions = drain_actions(create_pbft_manager_runtime_session(fact(
            PbftManagerRuntimeStateCode::ValueProposal,
        )));

        assert_eq!(
            actions,
            vec![
                PbftManagerRuntimeAction::ProcessSyncedPbftBlocks,
                PbftManagerRuntimeAction::MaybeBroadcastVotes,
                PbftManagerRuntimeAction::TryPushCertVotesBlock,
                PbftManagerRuntimeAction::TryAdvanceRound,
                PbftManagerRuntimeAction::RunValueProposal,
                PbftManagerRuntimeAction::TransitionToFilter,
                PbftManagerRuntimeAction::SleepUntilNextStep,
            ]
        );
    }

    #[test]
    fn network_syncing_skips_broadcast_and_cert_push_but_keeps_round_check() {
        let mut tick = fact(PbftManagerRuntimeStateCode::Filter);
        tick.network_pbft_syncing = true;
        let actions = drain_actions(create_pbft_manager_runtime_session(tick));

        assert_eq!(
            actions,
            vec![
                PbftManagerRuntimeAction::ProcessSyncedPbftBlocks,
                PbftManagerRuntimeAction::TryAdvanceRound,
                PbftManagerRuntimeAction::RunFilter,
                PbftManagerRuntimeAction::TransitionToCertify,
                PbftManagerRuntimeAction::SleepUntilNextStep,
            ]
        );
    }

    #[test]
    fn cert_push_progress_completes_with_restart_loop() {
        let mut session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Filter));
        for expected in [
            PbftManagerRuntimeAction::ProcessSyncedPbftBlocks,
            PbftManagerRuntimeAction::MaybeBroadcastVotes,
        ] {
            let step = next_pbft_manager_runtime_action(&session);
            assert_eq!(step.action, Some(expected));
            session = report_pbft_manager_runtime_action(session, report(step.cursor, expected));
        }

        let step = next_pbft_manager_runtime_action(&session);
        assert_eq!(
            step.action,
            Some(PbftManagerRuntimeAction::TryPushCertVotesBlock)
        );
        let mut action_report =
            report(step.cursor, PbftManagerRuntimeAction::TryPushCertVotesBlock);
        action_report.result = PbftManagerRuntimeActionResultCode::ProgressRestartLoop;
        session = report_pbft_manager_runtime_action(session, action_report);

        let final_step = next_pbft_manager_runtime_action(&session);
        assert!(final_step.complete);
        assert!(final_step.restart_loop);
    }

    #[test]
    fn advance_round_candidate_emits_reset_effect_and_restarts_after_report() {
        let mut session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Filter));
        for expected in [
            PbftManagerRuntimeAction::ProcessSyncedPbftBlocks,
            PbftManagerRuntimeAction::MaybeBroadcastVotes,
            PbftManagerRuntimeAction::TryPushCertVotesBlock,
        ] {
            let step = next_pbft_manager_runtime_action(&session);
            assert_eq!(step.action, Some(expected));
            let mut action_report = report(step.cursor, expected);
            if expected == PbftManagerRuntimeAction::TryPushCertVotesBlock {
                action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
            }
            session = report_pbft_manager_runtime_action(session, action_report);
        }

        let step = next_pbft_manager_runtime_action(&session);
        assert_eq!(step.action, Some(PbftManagerRuntimeAction::TryAdvanceRound));
        let mut action_report = report(step.cursor, PbftManagerRuntimeAction::TryAdvanceRound);
        action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
        action_report.has_new_round = true;
        action_report.new_round = 5;
        session = report_pbft_manager_runtime_action(session, action_report);

        let reset = next_pbft_manager_runtime_action(&session);
        assert_eq!(reset.action, Some(PbftManagerRuntimeAction::ResetConsensus));
        assert!(reset.has_target_round);
        assert_eq!(reset.target_round, 5);

        let mut reset_report = report(reset.cursor, PbftManagerRuntimeAction::ResetConsensus);
        reset_report.result = PbftManagerRuntimeActionResultCode::TransitionApplied;
        session = report_pbft_manager_runtime_action(session, reset_report);
        let complete = next_pbft_manager_runtime_action(&session);
        assert!(complete.complete);
        assert!(complete.restart_loop);
    }

    #[test]
    fn advance_round_rejects_non_increasing_candidate() {
        let mut session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Filter));
        for expected in [
            PbftManagerRuntimeAction::ProcessSyncedPbftBlocks,
            PbftManagerRuntimeAction::MaybeBroadcastVotes,
            PbftManagerRuntimeAction::TryPushCertVotesBlock,
        ] {
            let step = next_pbft_manager_runtime_action(&session);
            assert_eq!(step.action, Some(expected));
            let mut action_report = report(step.cursor, expected);
            if expected == PbftManagerRuntimeAction::TryPushCertVotesBlock {
                action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
            }
            session = report_pbft_manager_runtime_action(session, action_report);
        }

        let step = next_pbft_manager_runtime_action(&session);
        let mut action_report = report(step.cursor, PbftManagerRuntimeAction::TryAdvanceRound);
        action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
        action_report.has_new_round = true;
        action_report.new_round = 2;
        session = report_pbft_manager_runtime_action(session, action_report);

        let failed = next_pbft_manager_runtime_action(&session);
        assert_eq!(failed.status, PbftManagerRuntimeStatus::InvalidReport);
        assert_eq!(
            failed.error_code,
            "PBFT_MANAGER_ADVANCE_ROUND_NON_INCREASING_ROUND"
        );
    }

    #[test]
    fn ineligible_wallet_path_sleeps_and_restarts_without_state_action() {
        let mut tick = fact(PbftManagerRuntimeStateCode::ValueProposal);
        tick.has_eligible_wallet = false;
        let mut session = create_pbft_manager_runtime_session(tick);

        loop {
            let step = next_pbft_manager_runtime_action(&session);
            if step.action == Some(PbftManagerRuntimeAction::SleepIneligiblePollingInterval) {
                let mut action_report = report(
                    step.cursor,
                    PbftManagerRuntimeAction::SleepIneligiblePollingInterval,
                );
                action_report.result = PbftManagerRuntimeActionResultCode::SleepApplied;
                session = report_pbft_manager_runtime_action(session, action_report);
                break;
            }
            let action = step.action.expect("action");
            let mut action_report = report(step.cursor, action);
            if matches!(
                action,
                PbftManagerRuntimeAction::TryPushCertVotesBlock
                    | PbftManagerRuntimeAction::TryAdvanceRound
            ) {
                action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
            }
            if action == PbftManagerRuntimeAction::TryAdvanceRound {
                action_report.has_eligible_wallet = false;
            }
            session = report_pbft_manager_runtime_action(session, action_report);
        }

        let final_step = next_pbft_manager_runtime_action(&session);
        assert!(final_step.complete);
        assert!(final_step.restart_loop);
    }

    #[test]
    fn certify_branch_uses_reported_go_finish_flag() {
        let mut session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Certify));
        loop {
            let step = next_pbft_manager_runtime_action(&session);
            let action = step.action.expect("action");
            if action == PbftManagerRuntimeAction::RunCertify {
                let mut action_report = report(step.cursor, action);
                action_report.go_finish_state = true;
                session = report_pbft_manager_runtime_action(session, action_report);
                break;
            }
            let mut action_report = report(step.cursor, action);
            if matches!(
                action,
                PbftManagerRuntimeAction::TryPushCertVotesBlock
                    | PbftManagerRuntimeAction::TryAdvanceRound
            ) {
                action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
            }
            session = report_pbft_manager_runtime_action(session, action_report);
        }

        let step = next_pbft_manager_runtime_action(&session);
        assert_eq!(
            step.action,
            Some(PbftManagerRuntimeAction::TransitionToFinish)
        );
    }

    #[test]
    fn second_finish_branch_uses_reported_loopback_flag() {
        let mut session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::FinishPolling));
        loop {
            let step = next_pbft_manager_runtime_action(&session);
            let action = step.action.expect("action");
            if action == PbftManagerRuntimeAction::RunSecondFinish {
                let mut action_report = report(step.cursor, action);
                action_report.loop_back_finish_state = true;
                session = report_pbft_manager_runtime_action(session, action_report);
                break;
            }
            let mut action_report = report(step.cursor, action);
            if matches!(
                action,
                PbftManagerRuntimeAction::TryPushCertVotesBlock
                    | PbftManagerRuntimeAction::TryAdvanceRound
            ) {
                action_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
            }
            session = report_pbft_manager_runtime_action(session, action_report);
        }

        let step = next_pbft_manager_runtime_action(&session);
        assert_eq!(step.action, Some(PbftManagerRuntimeAction::LoopBackFinish));
    }

    #[test]
    fn cursor_mismatch_and_unknown_state_are_explicit_errors() {
        let session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Unknown));
        let step = next_pbft_manager_runtime_action(&session);
        assert_eq!(step.status, PbftManagerRuntimeStatus::RejectedTick);
        assert_eq!(step.error_code, "PBFT_MANAGER_RUNTIME_UNKNOWN_STATE");

        let session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Filter));
        let step = next_pbft_manager_runtime_action(&session);
        let mut bad_report = report(step.cursor + 1, step.action.expect("action"));
        bad_report.result = PbftManagerRuntimeActionResultCode::NoProgressContinue;
        let session = report_pbft_manager_runtime_action(session, bad_report);
        assert_eq!(session.status, PbftManagerRuntimeStatus::ActionMismatch);
    }

    #[test]
    fn runtime_rejects_unknown_action_and_result_codes() {
        let session =
            create_pbft_manager_runtime_session(fact(PbftManagerRuntimeStateCode::Filter));
        let step = next_pbft_manager_runtime_action(&session);

        let mut bad_action = report(step.cursor, PbftManagerRuntimeAction::Unknown);
        bad_action.result = PbftManagerRuntimeActionResultCode::StateActionDone;
        let failed = report_pbft_manager_runtime_action(session.clone(), bad_action);
        assert_eq!(failed.status, PbftManagerRuntimeStatus::ActionMismatch);

        let mut bad_result = report(step.cursor, step.action.expect("action"));
        bad_result.result = PbftManagerRuntimeActionResultCode::Unknown;
        let failed = report_pbft_manager_runtime_action(session, bad_result);
        assert_eq!(failed.status, PbftManagerRuntimeStatus::InvalidReport);
        assert_eq!(failed.error_code, "PBFT_MANAGER_RUNTIME_RESULT_MISMATCH");
    }

    #[test]
    fn state_action_planner_selects_value_proposal_starting_value() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::ValueProposal);
        fact.has_previous_round_next_null = true;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(plan.status, PbftManagerStateActionStatus::Ready);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::ProposeNewBlock
        );

        fact.has_previous_round_next_null = false;
        fact.has_previous_round_next_value = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::ReproposePreviousRoundNextValue
        );
        assert_eq!(plan.primary_hash, [0x11; 32]);
    }

    #[test]
    fn state_action_planner_selects_filter_branches() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::Filter);
        fact.round = 1;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::IdentifyLeaderAndSoftVote
        );

        fact.round = 2;
        fact.has_previous_round_next_value = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::SoftVotePreviousRoundNextValue
        );
        assert_eq!(plan.primary_hash, [0x11; 32]);
    }

    #[test]
    fn state_action_planner_selects_certify_timeout_and_vote() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::Certify);
        fact.elapsed_round_ms = 950;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(plan.primary_intent, PbftManagerStateActionIntent::GoFinish);
        assert!(plan.go_finish_state);

        fact.elapsed_round_ms = 250;
        fact.has_current_round_soft_value = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::CertVoteCurrentSoftValue
        );
        assert_eq!(plan.primary_hash, [0x22; 32]);
    }

    #[test]
    fn state_action_planner_selects_finish_votes() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::Finish);
        fact.has_cert_voted_block = true;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::NextVoteCertVotedBlock
        );
        assert_eq!(plan.primary_hash, [0x33; 32]);

        fact.has_cert_voted_block = false;
        fact.has_previous_round_next_null = true;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::NextVoteNullBlock
        );

        fact.has_previous_round_next_null = false;
        fact.has_previous_round_next_value = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::NextVotePreviousRoundValue
        );
        assert_eq!(plan.primary_hash, [0x11; 32]);
    }

    #[test]
    fn state_action_planner_selects_second_finish_primary_secondary_and_loopback() {
        let mut fact = state_fact(PbftManagerRuntimeStateCode::FinishPolling);
        fact.current_round_lambda_ms = 1_000;
        fact.has_current_round_soft_value = true;
        fact.has_previous_round_next_null = true;
        fact.elapsed_round_ms = 50;
        let plan = plan_pbft_manager_state_action(fact.clone());
        assert_eq!(
            plan.primary_intent,
            PbftManagerStateActionIntent::NextVoteCurrentSoftValue
        );
        assert_eq!(
            plan.secondary_intent,
            PbftManagerStateActionIntent::NextVoteNullBlock
        );
        assert!(!plan.loop_back_finish_state);

        fact.elapsed_round_ms = 2_000;
        fact.already_next_voted_value = true;
        fact.already_next_voted_null = true;
        let plan = plan_pbft_manager_state_action(fact);
        assert_eq!(plan.primary_intent, PbftManagerStateActionIntent::Noop);
        assert_eq!(plan.secondary_intent, PbftManagerStateActionIntent::Noop);
        assert!(plan.loop_back_finish_state);
    }

    #[test]
    fn startup_restore_normalizes_cursor_and_restores_status_flags() {
        let snapshot = restore_pbft_manager_runtime(startup_fact(2, 2));

        assert_eq!(snapshot.status, PbftManagerStartupRestoreStatus::Ready);
        assert_eq!(snapshot.state, PbftManagerRuntimeStateCode::Finish);
        assert_eq!(snapshot.round, 2);
        assert_eq!(snapshot.step, 4);
        assert_eq!(snapshot.current_round_lambda_ms, 500);
        assert_eq!(snapshot.rounds_count_dynamic_lambda, 7);
        assert_eq!(snapshot.dynamic_lambda_ms, 1_500);
        assert!(snapshot.executed_pbft_block);
        assert!(snapshot.already_next_voted_value);
        assert!(!snapshot.already_next_voted_null);
        assert!(snapshot.persist_normalized_step);
    }

    #[test]
    fn startup_restore_maps_scratch_and_finish_polling_states() {
        let scratch = restore_pbft_manager_runtime(startup_fact(1, 1));
        assert_eq!(scratch.state, PbftManagerRuntimeStateCode::ValueProposal);
        assert_eq!(scratch.step, 1);
        assert!(!scratch.persist_normalized_step);
        assert!(!scratch.reset_second_finish_start);

        let polling = restore_pbft_manager_runtime(startup_fact(4, 5));
        assert_eq!(polling.state, PbftManagerRuntimeStateCode::FinishPolling);
        assert_eq!(polling.step, 5);
        assert!(polling.reset_second_finish_start);
    }

    #[test]
    fn startup_restore_rejects_missing_cacti_dynamic_lambda() {
        let mut fact = startup_fact(1, 1);
        fact.persisted_dynamic_lambda_ms = 1;
        let snapshot = restore_pbft_manager_runtime(fact);

        assert_eq!(
            snapshot.status,
            PbftManagerStartupRestoreStatus::InvalidFact
        );
        assert_eq!(
            snapshot.error_code,
            "PBFT_MANAGER_STARTUP_MISSING_DYNAMIC_LAMBDA"
        );
    }

    #[test]
    fn storage_startup_restore_reads_rust_storage_and_persists_normalized_step() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_storage_startup");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_ROUND, 2)
                .expect("round should persist");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_STEP, 2)
                .expect("step should persist");
            storage
                .pbft()
                .write_manager_status(PBFT_MGR_STATUS_EXECUTED_BLOCK, true)
                .expect("executed status should persist");
            storage
                .pbft()
                .write_manager_status(PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE, true)
                .expect("next-voted status should persist");

            let runtime =
                create_pbft_manager_runtime_from_storage(&storage, storage_startup_fact())
                    .expect("runtime should restore from Rust storage");
            let snapshot = runtime.snapshot();

            assert_eq!(snapshot.status, PbftManagerStartupRestoreStatus::Ready);
            assert_eq!(snapshot.state, PbftManagerRuntimeStateCode::Finish);
            assert_eq!(snapshot.round, 2);
            assert_eq!(snapshot.step, 4);
            assert_eq!(snapshot.current_round_lambda_ms, 1_000);
            assert!(snapshot.executed_pbft_block);
            assert!(snapshot.already_next_voted_value);
            assert!(!snapshot.already_next_voted_null);
            assert!(!snapshot.persist_normalized_step);
            assert_eq!(
                storage
                    .pbft()
                    .manager_field(PBFT_MGR_FIELD_STEP)
                    .expect("normalized step should load"),
                Some(4),
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn runtime_cursor_field_storage_persists_round_and_step_only() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_cursor_field");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_ROUND, 1)
                .expect("round seed should persist");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_STEP, 1)
                .expect("step seed should persist");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_LAMBDA, 1_500)
                .expect("lambda seed should persist");
            let mut runtime =
                create_pbft_manager_runtime_from_storage(&storage, storage_startup_fact())
                    .expect("runtime should restore from Rust storage");

            apply_pbft_manager_cursor_field_storage(&storage, PBFT_MGR_FIELD_ROUND, 7)
                .expect("round cursor should persist");
            runtime.apply_committed_cursor_field(PBFT_MGR_FIELD_ROUND, 7);
            apply_pbft_manager_cursor_field_storage(&storage, PBFT_MGR_FIELD_STEP, 9)
                .expect("step cursor should persist");
            runtime.apply_committed_cursor_field(PBFT_MGR_FIELD_STEP, 9);
            let err = apply_pbft_manager_cursor_field_storage(&storage, PBFT_MGR_FIELD_LAMBDA, 1)
                .expect_err("dynamic lambda should not use cursor field API");

            let snapshot = runtime.snapshot();
            assert_eq!(snapshot.round, 7);
            assert_eq!(snapshot.step, 9);
            assert_eq!(
                storage
                    .pbft()
                    .manager_field(PBFT_MGR_FIELD_ROUND)
                    .expect("round should load"),
                Some(7),
            );
            assert_eq!(
                storage
                    .pbft()
                    .manager_field(PBFT_MGR_FIELD_STEP)
                    .expect("step should load"),
                Some(9),
            );
            assert!(
                err.to_string()
                    .contains("unsupported PBFT manager cursor field")
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn cert_voted_block_storage_write_persists_legacy_payload() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_cert_voted_write");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");

            save_cert_voted_block_in_round_storage(&storage, 5, &[0xC0])
                .expect("cert-voted block should persist");
            let err = save_cert_voted_block_in_round_storage(&storage, 6, &[])
                .expect_err("empty PBFT block payload should reject");

            let payload = storage
                .pbft()
                .cert_voted_block_in_round_rlp()
                .expect("cert-voted block should load")
                .expect("cert-voted block should exist");
            let rlp = rlp::Rlp::new(&payload);
            assert_eq!(rlp.item_count().unwrap(), 2);
            assert_eq!(rlp.at(0).unwrap().as_val::<u64>().unwrap(), 5);
            assert_eq!(rlp.at(1).unwrap().as_raw(), &[0xC0]);
            assert_eq!(
                err.to_string(),
                "PBFT_MANAGER_CERT_VOTED_BLOCK_EMPTY_PAYLOAD"
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn startup_replay_period_loader_reads_period_lambda_and_finalized_dag_hashes() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_startup_replay");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            let (bundle, expected_hashes) = finalized_dag_bundle_rlp();
            let period_data = period_data_with_dag_bundle(&bundle);
            storage
                .period()
                .write(12, &period_data)
                .expect("period data should persist");
            storage
                .metadata()
                .write_period_lambda(11, 1_234)
                .expect("period lambda should persist");

            let replay = load_pbft_manager_startup_replay_period(&storage, 12, true)
                .expect("startup replay period should load");

            assert!(replay.found);
            assert_eq!(replay.period_data_rlp, period_data);
            assert_eq!(replay.finalized_dag_hashes, expected_hashes);
            assert_eq!(replay.period_lambda, Some(1_234));
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn startup_replay_period_loader_reports_missing_period_data_without_fallback() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_startup_replay_missing");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");

            let replay = load_pbft_manager_startup_replay_period(&storage, 99, true)
                .expect("missing startup replay period should be explicit");

            assert!(!replay.found);
            assert!(replay.period_data_rlp.is_empty());
            assert!(replay.finalized_dag_hashes.is_empty());
            assert_eq!(replay.period_lambda, None);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn storage_startup_restore_rejects_corrupt_rust_storage() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_storage_corrupt_startup");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_ROUND, 0)
                .expect("corrupt round should persist");

            let err = create_pbft_manager_runtime_from_storage(&storage, storage_startup_fact())
                .expect_err("corrupt cursor should reject startup");
            assert_eq!(err.to_string(), "PBFT_MANAGER_STARTUP_INVALID_CURSOR");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn transition_storage_apply_commits_manager_status_and_own_vote_cleanup() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_transition_storage");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            let own_hash = H256::from([0xAB; 32]);
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_ROUND, 1)
                .expect("round seed should persist");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_STEP, 1)
                .expect("step seed should persist");
            storage
                .pbft()
                .write_manager_status(PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE, true)
                .expect("soft next status should persist");
            storage
                .pbft()
                .write_manager_status(PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH, true)
                .expect("null next status should persist");
            storage
                .pbft()
                .write_cert_voted_block_in_round(1, &[0xC0])
                .expect("cert-voted seed should persist");
            storage
                .pbft()
                .write_own_verified_vote(own_hash, &[0xC1])
                .expect("own vote should persist");

            let mut plan = plan_pbft_manager_transition(transition_fact(
                PbftManagerTransitionKind::ResetConsensus,
            ));
            plan.remove_cert_voted_block = true;
            let result = apply_pbft_manager_transition_storage(&storage, &plan, &[own_hash], false)
                .expect("transition storage should return a result");

            assert_eq!(result.status, PbftManagerTransitionStorageStatus::Applied);
            assert_eq!(result.applied_writes, 6);
            assert_eq!(
                storage
                    .pbft()
                    .manager_field(PBFT_MGR_FIELD_ROUND)
                    .expect("round should load"),
                Some(4),
            );
            assert_eq!(
                storage
                    .pbft()
                    .manager_field(PBFT_MGR_FIELD_STEP)
                    .expect("step should load"),
                Some(1),
            );
            assert_eq!(
                storage
                    .pbft()
                    .manager_status(PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE)
                    .expect("soft next status should load"),
                Some(false),
            );
            assert_eq!(
                storage
                    .pbft()
                    .manager_status(PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH)
                    .expect("null next status should load"),
                Some(false),
            );
            assert!(
                storage
                    .pbft()
                    .cert_voted_block_in_round_rlp()
                    .expect("cert-voted block should load")
                    .is_none()
            );
            assert!(
                storage
                    .pbft()
                    .own_verified_votes_rlp()
                    .expect("own votes should load")
                    .is_empty()
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn transition_storage_rejects_unexpected_own_vote_hash_without_mutation() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_transition_storage_reject");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");
            storage
                .pbft()
                .write_manager_field(PBFT_MGR_FIELD_ROUND, 3)
                .expect("round seed should persist");

            let mut plan =
                plan_pbft_manager_transition(transition_fact(PbftManagerTransitionKind::ToFilter));
            plan.clear_own_votes = false;
            let result = apply_pbft_manager_transition_storage(
                &storage,
                &plan,
                &[H256::from([0xCD; 32])],
                false,
            )
            .expect("transition storage should return a rejection");

            assert_eq!(result.status, PbftManagerTransitionStorageStatus::Rejected);
            assert_eq!(
                result.error_code,
                "PBFT_MANAGER_TRANSITION_STORAGE_UNEXPECTED_OWN_VOTE_HASHES"
            );
            assert_eq!(
                storage
                    .pbft()
                    .manager_field(PBFT_MGR_FIELD_ROUND)
                    .expect("round should load"),
                Some(3),
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn next_voted_status_storage_persists_only_next_vote_family() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pbft_manager_next_voted_status");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");

            apply_next_voted_status_storage(&storage, PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE)
                .expect("soft next-voted status should persist");
            apply_next_voted_status_storage(&storage, PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH)
                .expect("null next-voted status should persist");
            let err = apply_next_voted_status_storage(&storage, PBFT_MGR_STATUS_EXECUTED_BLOCK)
                .expect_err("generic PBFT manager status should reject");

            assert_eq!(
                storage
                    .pbft()
                    .manager_status(PBFT_MGR_STATUS_NEXT_VOTED_SOFT_VALUE)
                    .expect("soft status should load"),
                Some(true),
            );
            assert_eq!(
                storage
                    .pbft()
                    .manager_status(PBFT_MGR_STATUS_NEXT_VOTED_NULL_BLOCK_HASH)
                    .expect("null status should load"),
                Some(true),
            );
            assert_eq!(
                err.to_string(),
                "PBFT_MANAGER_NEXT_VOTED_STATUS_UNSUPPORTED"
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn runtime_snapshot_advances_only_after_committed_transition_report() {
        let mut runtime = PbftManagerRuntime::new(restore_pbft_manager_runtime(startup_fact(2, 4)));
        let before = runtime.snapshot();
        let rejected = reject_transition_plan(
            PbftManagerTransitionStatus::InvalidFact,
            PbftManagerTransitionKind::ToFilter,
            "rejected",
        );
        runtime.apply_committed_transition(&rejected);
        assert_eq!(runtime.snapshot(), before);

        let plan =
            plan_pbft_manager_transition(transition_fact(PbftManagerTransitionKind::ToFilter));
        runtime.apply_committed_transition(&plan);
        let after = runtime.snapshot();
        assert_eq!(after.state, PbftManagerRuntimeStateCode::Filter);
        assert_eq!(after.round, 2);
        assert_eq!(after.step, 4);
        assert_eq!(after.current_round_lambda_ms, 100);
        assert_eq!(after.next_step_time_ms, 200);
    }

    #[test]
    fn runtime_snapshot_records_committed_executed_block_reset() {
        let mut runtime = PbftManagerRuntime::new(restore_pbft_manager_runtime(startup_fact(2, 4)));
        assert!(runtime.snapshot().executed_pbft_block);

        runtime.apply_committed_executed_block_reset();
        let after = runtime.snapshot();

        assert_eq!(after.status, PbftManagerStartupRestoreStatus::Ready);
        assert!(!after.executed_pbft_block);
        assert!(after.error_code.is_empty());
    }

    #[test]
    fn transition_planner_selects_phase_targets_and_timing() {
        let filter =
            plan_pbft_manager_transition(transition_fact(PbftManagerTransitionKind::ToFilter));
        assert_eq!(filter.status, PbftManagerTransitionStatus::Ready);
        assert_eq!(filter.new_state, PbftManagerRuntimeStateCode::Filter);
        assert_eq!(filter.new_round, 2);
        assert_eq!(filter.new_step, 4);
        assert_eq!(filter.current_round_lambda_ms, 100);
        assert_eq!(filter.next_step_time_ms, 200);
        assert!(filter.persist_step);

        let certify =
            plan_pbft_manager_transition(transition_fact(PbftManagerTransitionKind::ToCertify));
        assert_eq!(certify.new_state, PbftManagerRuntimeStateCode::Certify);
        assert!(certify.print_cert_step_info);

        let finish =
            plan_pbft_manager_transition(transition_fact(PbftManagerTransitionKind::ToFinish));
        assert_eq!(finish.new_state, PbftManagerRuntimeStateCode::Finish);
        assert_eq!(finish.next_step_time_ms, 1_000);

        let finish_polling = plan_pbft_manager_transition(transition_fact(
            PbftManagerTransitionKind::ToFinishPolling,
        ));
        assert_eq!(
            finish_polling.new_state,
            PbftManagerRuntimeStateCode::FinishPolling
        );
        assert_eq!(finish_polling.next_step_time_ms, 1_000);
        assert!(finish_polling.reset_next_voted_statuses);
        assert!(finish_polling.reset_second_finish_start);
        assert!(finish_polling.print_second_finish_step_info);

        let delay_certify = plan_pbft_manager_transition(transition_fact(
            PbftManagerTransitionKind::DelayCertifyPoll,
        ));
        assert_eq!(
            delay_certify.new_state,
            PbftManagerRuntimeStateCode::Certify
        );
        assert_eq!(delay_certify.new_step, 3);
        assert_eq!(delay_certify.next_step_time_ms, 1_000);
        assert!(!delay_certify.persist_step);

        let delay_finish = plan_pbft_manager_transition(transition_fact(
            PbftManagerTransitionKind::DelayFinishPoll,
        ));
        assert_eq!(
            delay_finish.new_state,
            PbftManagerRuntimeStateCode::FinishPolling
        );
        assert_eq!(delay_finish.new_step, 3);
        assert_eq!(delay_finish.next_step_time_ms, 1_000);
        assert!(!delay_finish.persist_step);
    }

    #[test]
    fn transition_planner_selects_reset_effects() {
        let reset = plan_pbft_manager_transition(transition_fact(
            PbftManagerTransitionKind::ResetConsensus,
        ));
        assert_eq!(reset.status, PbftManagerTransitionStatus::Ready);
        assert_eq!(reset.new_state, PbftManagerRuntimeStateCode::ValueProposal);
        assert_eq!(reset.new_round, 4);
        assert_eq!(reset.new_step, 1);
        assert_eq!(reset.current_round_lambda_ms, 400);
        assert!(reset.persist_round);
        assert!(reset.persist_step);
        assert!(reset.reset_next_voted_statuses);
        assert!(reset.remove_cert_voted_block);
        assert!(reset.clear_own_votes);
        assert!(reset.clear_broadcasted_votes);
        assert!(reset.reset_broadcast_counters);
        assert!(reset.reset_executed_block_status);
        assert!(reset.set_vote_manager_period_round);
        assert!(reset.reset_current_round_start);
    }

    #[test]
    fn transition_planner_applies_finish_loopback_and_lambda_backoff() {
        let mut fact = transition_fact(PbftManagerTransitionKind::LoopBackFinish);
        fact.step = 12;
        fact.current_round_lambda_ms = 100;
        fact.next_step_time_ms = 900;
        let plan = plan_pbft_manager_transition(fact);

        assert_eq!(plan.status, PbftManagerTransitionStatus::Ready);
        assert_eq!(plan.new_state, PbftManagerRuntimeStateCode::Finish);
        assert_eq!(plan.new_step, 13);
        assert_eq!(plan.current_round_lambda_ms, 200);
        assert_eq!(plan.next_step_time_ms, 1_000);
        assert!(plan.reset_next_voted_statuses);
    }

    #[test]
    fn transition_planner_resets_lambda_when_network_is_far_ahead() {
        let mut fact = transition_fact(PbftManagerTransitionKind::LoopBackFinish);
        fact.step = 14;
        fact.current_round_lambda_ms = 800;
        fact.network_next_voting_step = 24;
        let plan = plan_pbft_manager_transition(fact);

        assert_eq!(plan.new_step, 15);
        assert_eq!(plan.current_round_lambda_ms, 100);
    }

    #[test]
    fn transition_and_advance_planners_reject_invalid_facts() {
        let mut invalid = transition_fact(PbftManagerTransitionKind::ToFilter);
        invalid.step = 0;
        let plan = plan_pbft_manager_transition(invalid);
        assert_eq!(plan.status, PbftManagerTransitionStatus::InvalidFact);
        assert_eq!(plan.error_code, "PBFT_MANAGER_TRANSITION_INVALID_CURSOR");

        let no_candidate = plan_pbft_manager_advance_round(PbftManagerAdvanceRoundFact {
            period: 10,
            current_round: 2,
            has_new_round: false,
            new_round: 0,
        });
        assert_eq!(no_candidate.status, PbftManagerTransitionStatus::Ready);
        assert!(!no_candidate.should_advance);

        let invalid_round = plan_pbft_manager_advance_round(PbftManagerAdvanceRoundFact {
            period: 10,
            current_round: 2,
            has_new_round: true,
            new_round: 2,
        });
        assert_eq!(
            invalid_round.status,
            PbftManagerTransitionStatus::InvalidFact
        );
        assert_eq!(
            invalid_round.error_code,
            "PBFT_MANAGER_ADVANCE_ROUND_NON_INCREASING_ROUND"
        );
    }

    #[test]
    fn leader_selection_prefers_lowest_ranked_non_null_candidate() {
        let mut high_rank = leader_candidate(1, 1, PbftManagerLeaderCandidateStatus::Ready, 9);
        high_rank.weight = 2;
        let low_rank = leader_candidate(2, 2, PbftManagerLeaderCandidateStatus::Ready, 10);
        let null_anchor = leader_candidate(3, 3, PbftManagerLeaderCandidateStatus::Ready, 0);

        let plan = plan_pbft_manager_leader_selection(vec![
            high_rank.clone(),
            low_rank.clone(),
            null_anchor,
        ]);
        assert_eq!(plan.status, PbftManagerLeaderSelectionStatus::Selected);
        assert!(plan.selected);
        assert!(!plan.selected_from_null_anchor);

        let high_rank_hash = pbft_manager_proposal_rank_hash(
            high_rank.credential,
            high_rank.voter_public_key,
            high_rank.weight,
        )
        .unwrap();
        let low_rank_hash =
            pbft_manager_proposal_rank_hash(low_rank.credential, low_rank.voter_public_key, 1)
                .unwrap();
        let expected = if high_rank_hash < low_rank_hash {
            high_rank
        } else {
            low_rank
        };
        assert_eq!(plan.selected_vote_hash, expected.vote_hash);
        assert_eq!(plan.selected_block_hash, expected.block_hash);
    }

    #[test]
    fn leader_selection_uses_null_anchor_only_as_fallback() {
        let invalid = leader_candidate(
            1,
            1,
            PbftManagerLeaderCandidateStatus::BlockMissingOrInvalid,
            8,
        );
        let in_chain = leader_candidate(2, 2, PbftManagerLeaderCandidateStatus::BlockInChain, 9);
        let null_anchor = leader_candidate(3, 3, PbftManagerLeaderCandidateStatus::Ready, 0);

        let plan = plan_pbft_manager_leader_selection(vec![invalid, null_anchor.clone(), in_chain]);
        assert_eq!(plan.status, PbftManagerLeaderSelectionStatus::Selected);
        assert!(plan.selected_from_null_anchor);
        assert_eq!(plan.selected_vote_hash, null_anchor.vote_hash);
    }

    #[test]
    fn leader_selection_keeps_last_duplicate_rank_candidate() {
        let first = leader_candidate(1, 1, PbftManagerLeaderCandidateStatus::Ready, 5);
        let mut second = leader_candidate(2, 2, PbftManagerLeaderCandidateStatus::Ready, 6);
        second.credential = first.credential;
        second.voter_public_key = first.voter_public_key;
        second.weight = first.weight;

        let plan = plan_pbft_manager_leader_selection(vec![first, second.clone()]);
        assert_eq!(plan.status, PbftManagerLeaderSelectionStatus::Selected);
        assert_eq!(plan.selected_vote_hash, second.vote_hash);
    }

    #[test]
    fn leader_selection_rejects_unknown_status_and_skips_invalid_weight() {
        let unknown = leader_candidate(1, 1, PbftManagerLeaderCandidateStatus::Unknown, 1);
        let plan = plan_pbft_manager_leader_selection(vec![unknown]);
        assert_eq!(plan.status, PbftManagerLeaderSelectionStatus::InvalidFact);

        let zero_weight = leader_candidate(2, 2, PbftManagerLeaderCandidateStatus::Ready, 2);
        let plan = plan_pbft_manager_leader_selection(vec![PbftManagerLeaderCandidateFact {
            weight: 0,
            ..zero_weight
        }]);
        assert_eq!(
            plan.status,
            PbftManagerLeaderSelectionStatus::NoEligibleCandidate
        );
    }

    #[test]
    fn leader_candidate_planner_derives_statuses_and_mark_valid_commands() {
        let invalid_weight = leader_candidate_input(1, 1);
        let in_chain = PbftManagerLeaderCandidateInputFact {
            block_in_chain: true,
            ..leader_candidate_input(2, 2)
        };
        let missing = PbftManagerLeaderCandidateInputFact {
            proposed_block_found: false,
            ..leader_candidate_input(3, 3)
        };
        let valid = PbftManagerLeaderCandidateInputFact {
            block_validation_status: PbftManagerLeaderBlockValidationStatus::Validated,
            pivot_hash: H256::from([9; 32]),
            ..leader_candidate_input(4, 4)
        };
        let mut invalid_weight = invalid_weight;
        invalid_weight.weight_found = false;

        let plan =
            plan_pbft_manager_leader_candidates(vec![invalid_weight, in_chain, missing, valid]);

        assert_eq!(plan.status, PbftManagerLeaderSelectionStatus::Selected);
        assert!(plan.selected);
        assert_eq!(plan.selected_vote_hash, H256::from([4; 32]));
        assert_eq!(plan.selected_block_hash, H256::from([4; 32]));
        assert_eq!(
            plan.valid_blocks,
            vec![PbftManagerLeaderValidBlockCommand {
                period: 7,
                block_hash: H256::from([4; 32]),
            }]
        );
    }

    #[test]
    fn leader_candidate_planner_keeps_already_valid_blocks_out_of_mark_commands() {
        let fallback = PbftManagerLeaderCandidateInputFact {
            block_validation_status: PbftManagerLeaderBlockValidationStatus::AlreadyValid,
            pivot_hash: H256::zero(),
            ..leader_candidate_input(1, 1)
        };
        let selected = PbftManagerLeaderCandidateInputFact {
            block_validation_status: PbftManagerLeaderBlockValidationStatus::AlreadyValid,
            pivot_hash: H256::from([8; 32]),
            ..leader_candidate_input(2, 2)
        };

        let plan = plan_pbft_manager_leader_candidates(vec![fallback, selected]);

        assert_eq!(plan.status, PbftManagerLeaderSelectionStatus::Selected);
        assert!(!plan.selected_from_null_anchor);
        assert!(plan.valid_blocks.is_empty());
        assert_eq!(plan.selected_block_hash, H256::from([2; 32]));
    }

    #[test]
    fn leader_candidate_planner_rejects_unknown_validation_status() {
        let plan = plan_pbft_manager_leader_candidates(vec![PbftManagerLeaderCandidateInputFact {
            block_validation_status: PbftManagerLeaderBlockValidationStatus::Unknown,
            ..leader_candidate_input(1, 1)
        }]);

        assert_eq!(plan.status, PbftManagerLeaderSelectionStatus::InvalidFact);
        assert_eq!(
            plan.error_code,
            "PBFT_MANAGER_LEADER_UNKNOWN_BLOCK_VALIDATION_STATUS"
        );
    }

    #[test]
    fn candidate_admission_plans_lookup_validation_and_mark_valid() {
        let mut fact = candidate_admission_fact();

        let plan = plan_pbft_manager_candidate_admission(fact.clone());
        assert_eq!(
            plan.action,
            PbftManagerCandidateAdmissionAction::RequestLookup
        );
        assert_eq!(
            plan.status,
            PbftManagerCandidateAdmissionStatus::LookupRequired
        );
        assert!(!plan.mark_valid);

        fact.lookup_performed = true;
        fact.proposed_block_found = true;
        let plan = plan_pbft_manager_candidate_admission(fact.clone());
        assert_eq!(
            plan.action,
            PbftManagerCandidateAdmissionAction::RequestValidation
        );
        assert_eq!(
            plan.status,
            PbftManagerCandidateAdmissionStatus::ValidationRequired
        );

        fact.validation_status = PbftManagerCandidateAdmissionValidationStatus::Valid;
        let plan = plan_pbft_manager_candidate_admission(fact);
        assert_eq!(plan.action, PbftManagerCandidateAdmissionAction::Accept);
        assert_eq!(
            plan.status,
            PbftManagerCandidateAdmissionStatus::AcceptedNewlyValidated
        );
        assert!(plan.mark_valid);
    }

    #[test]
    fn candidate_admission_accepts_already_valid_and_rejects_missing() {
        let already_valid = PbftManagerCandidateAdmissionFact {
            lookup_performed: true,
            proposed_block_found: true,
            proposed_block_already_valid: true,
            ..candidate_admission_fact()
        };
        let plan = plan_pbft_manager_candidate_admission(already_valid);
        assert_eq!(plan.action, PbftManagerCandidateAdmissionAction::Accept);
        assert_eq!(
            plan.status,
            PbftManagerCandidateAdmissionStatus::AcceptedAlreadyValid
        );
        assert!(!plan.mark_valid);

        let missing = PbftManagerCandidateAdmissionFact {
            lookup_performed: true,
            proposed_block_found: false,
            ..candidate_admission_fact()
        };
        let plan = plan_pbft_manager_candidate_admission(missing);
        assert_eq!(plan.action, PbftManagerCandidateAdmissionAction::Reject);
        assert_eq!(
            plan.status,
            PbftManagerCandidateAdmissionStatus::BlockMissing
        );
    }

    #[test]
    fn candidate_admission_rejects_bad_fact_order() {
        let bad = PbftManagerCandidateAdmissionFact {
            lookup_performed: false,
            proposed_block_found: true,
            ..candidate_admission_fact()
        };
        let plan = plan_pbft_manager_candidate_admission(bad);
        assert_eq!(
            plan.action,
            PbftManagerCandidateAdmissionAction::ContractError
        );
        assert_eq!(
            plan.status,
            PbftManagerCandidateAdmissionStatus::InvalidBridgeFacts
        );
    }

    #[test]
    fn block_validation_planner_drives_live_checks_in_legacy_order() {
        let mut fact = block_validation_fact();

        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(plan.action, PbftManagerBlockValidationAction::RunCheck);
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::CheckPbftChain
        );

        fact.pbft_chain_status = PbftManagerBlockValidationFactStatus::Valid;
        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::ValidateFinalChainHash
        );

        fact.final_chain_hash_status = PbftManagerBlockValidationFactStatus::Valid;
        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::CheckRewardVotes
        );

        fact.reward_votes_status = PbftManagerBlockValidationFactStatus::Valid;
        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::ValidateExtraData
        );

        fact.extra_data_status = PbftManagerBlockValidationFactStatus::Valid;
        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::CheckDagOrder
        );

        fact.dag_order_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.dag_weight_check_required = true;
        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::CheckDagWeight
        );

        fact.dag_weight_status = PbftManagerBlockValidationFactStatus::Valid;
        let plan = plan_pbft_manager_block_validation(fact);
        assert_eq!(plan.action, PbftManagerBlockValidationAction::Accept);
        assert_eq!(plan.status, PbftManagerBlockValidationStatus::Accepted);
    }

    #[test]
    fn block_validation_planner_handles_final_chain_wait_and_rejections() {
        let mut fact = block_validation_fact();
        fact.pbft_chain_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.final_chain_hash_status = PbftManagerBlockValidationFactStatus::Missing;
        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(
            plan.action,
            PbftManagerBlockValidationAction::WaitForFinalization
        );
        assert_eq!(
            plan.status,
            PbftManagerBlockValidationStatus::FinalChainHashMissing
        );

        fact.final_chain_hash_status = PbftManagerBlockValidationFactStatus::Invalid;
        let plan = plan_pbft_manager_block_validation(fact);
        assert_eq!(plan.action, PbftManagerBlockValidationAction::Reject);
        assert_eq!(
            plan.status,
            PbftManagerBlockValidationStatus::FinalChainHashInvalid
        );
    }

    #[test]
    fn block_validation_planner_accepts_null_or_cached_anchor_without_dag_checks() {
        let mut fact = block_validation_fact();
        fact.pbft_chain_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.final_chain_hash_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.reward_votes_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.extra_data_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.pivot_is_null = true;
        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(plan.action, PbftManagerBlockValidationAction::Accept);

        fact.pivot_is_null = false;
        fact.dag_order_cached = true;
        let plan = plan_pbft_manager_block_validation(fact);
        assert_eq!(plan.action, PbftManagerBlockValidationAction::Accept);
    }

    #[test]
    fn block_validation_planner_can_skip_dag_order_for_sync_context() {
        let mut fact = block_validation_fact();
        fact.pbft_chain_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.final_chain_hash_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.reward_votes_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.extra_data_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.dag_order_required = false;

        let plan = plan_pbft_manager_block_validation(fact);

        assert_eq!(plan.action, PbftManagerBlockValidationAction::Accept);
        assert_eq!(plan.status, PbftManagerBlockValidationStatus::Accepted);
    }

    #[test]
    fn block_validation_planner_requires_pillar_block_only_when_configured() {
        let mut fact = block_validation_fact();
        fact.pbft_chain_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.final_chain_hash_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.reward_votes_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.extra_data_status = PbftManagerBlockValidationFactStatus::Valid;
        fact.pillar_block_required = true;
        fact.pillar_block_status = PbftManagerBlockValidationFactStatus::NotChecked;

        let plan = plan_pbft_manager_block_validation(fact.clone());
        assert_eq!(
            plan.next_check,
            PbftManagerBlockValidationNextCheck::ValidatePillarBlock
        );

        fact.pillar_block_status = PbftManagerBlockValidationFactStatus::Invalid;
        let plan = plan_pbft_manager_block_validation(fact);
        assert_eq!(plan.action, PbftManagerBlockValidationAction::Reject);
        assert_eq!(
            plan.status,
            PbftManagerBlockValidationStatus::PillarBlockInvalid
        );
    }

    fn leader_candidate(
        id: u8,
        block: u8,
        status: PbftManagerLeaderCandidateStatus,
        pivot: u8,
    ) -> PbftManagerLeaderCandidateFact {
        PbftManagerLeaderCandidateFact {
            vote_hash: H256::from([id; 32]),
            block_hash: H256::from([block; 32]),
            period: 7,
            credential: [id; 64],
            voter_public_key: [id.wrapping_add(11); 64],
            weight: 1,
            status,
            pivot_hash: H256::from([pivot; 32]),
        }
    }

    fn leader_candidate_input(id: u8, block: u8) -> PbftManagerLeaderCandidateInputFact {
        PbftManagerLeaderCandidateInputFact {
            vote_hash: H256::from([id; 32]),
            block_hash: H256::from([block; 32]),
            period: 7,
            credential: [id; 64],
            voter_public_key: [id.wrapping_add(11); 64],
            weight_found: true,
            weight: 1,
            block_in_chain: false,
            proposed_block_found: true,
            block_validation_status: PbftManagerLeaderBlockValidationStatus::Validated,
            pivot_hash: H256::from([block.wrapping_add(20); 32]),
        }
    }

    fn candidate_admission_fact() -> PbftManagerCandidateAdmissionFact {
        PbftManagerCandidateAdmissionFact {
            period: 7,
            block_hash: H256::from([1; 32]),
            lookup_performed: false,
            proposed_block_found: false,
            proposed_block_already_valid: false,
            validation_status: PbftManagerCandidateAdmissionValidationStatus::NotChecked,
        }
    }

    fn block_validation_fact() -> PbftManagerBlockValidationFact {
        PbftManagerBlockValidationFact {
            block_hash: H256::from([1; 32]),
            period: 7,
            pivot_hash: H256::from([2; 32]),
            pivot_is_null: false,
            dag_order_cached: false,
            dag_order_required: true,
            pillar_block_required: false,
            dag_weight_check_required: false,
            pbft_chain_status: PbftManagerBlockValidationFactStatus::NotChecked,
            final_chain_hash_status: PbftManagerBlockValidationFactStatus::NotChecked,
            reward_votes_status: PbftManagerBlockValidationFactStatus::NotChecked,
            extra_data_status: PbftManagerBlockValidationFactStatus::NotChecked,
            pillar_block_status: PbftManagerBlockValidationFactStatus::NotRequired,
            dag_order_status: PbftManagerBlockValidationFactStatus::NotChecked,
            dag_weight_status: PbftManagerBlockValidationFactStatus::NotChecked,
        }
    }
}
