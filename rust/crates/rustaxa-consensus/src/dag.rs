use anyhow::{Context, Result, bail, ensure};
use ethereum_types::H256;
use rlp::Rlp;
use rustaxa_storage::{StatusField, Storage};
use rustaxa_types::codec::rlp::dag::DagBlockRlp;
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::dag::DagBlock;
use rustaxa_types::pbft::PbftBlockLink;
use rustaxa_vdf::sortition::{self, LegacySortitionParams};
use rustaxa_vdf::vdf::{Solution as VdfSolution, WesolowskiVdf};
use rustaxa_vdf::verifier::WesolowskiVerifier;
use rustaxa_vdf::{vdf_sortition, vrf};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write;

const PBFT_BLOCK_POS_IN_PERIOD_DATA: usize = 0;

/// Deterministic DAG frontier derived from a ghost path and DAG leaves.
///
/// Inputs:
/// - `pivot`: last hash in the ghost path (or zero hash when the path is empty).
/// - `tips`: leaf hashes excluding `pivot`.
///
/// Output invariants:
/// - `tips` never contains `pivot`.
/// - tip order is preserved from the input `leaves`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagFrontier {
    pub pivot: H256,
    pub tips: Vec<H256>,
}

/// Rust-owned DAG proposer graph facts for one proposal attempt.
///
/// Inputs:
/// - `frontier`: cached pivot and tips derived from the current Rust DAG graph.
/// - `propose_level`: next DAG level computed from Rust block-level metadata.
/// - `anchor`: current finalized DAG anchor used for non-finalized pressure gating.
/// - `non_finalized_block_count`: total live non-finalized DAG blocks.
/// - `non_finalized_min_difficulty`: minimum VDF difficulty among live non-finalized blocks, or `u32::MAX` when empty.
///
/// Invariants:
/// - `propose_level` is one greater than the highest available frontier reference level.
/// - Missing frontier metadata contributes level `0`, preserving legacy proposer behavior while keeping the lookup in
///   Rust-owned DAG state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagProposerFrontierFacts {
    pub frontier: DagFrontier,
    pub propose_level: u64,
    pub anchor: H256,
    pub non_finalized_block_count: usize,
    pub non_finalized_min_difficulty: u32,
}

/// Per-reference metadata used for pivot/tip level validation.
///
/// Inputs:
/// - `hash`: pivot/tip hash being validated.
/// - `found`: whether the reference block metadata exists.
/// - `level`: reference block level when `found == true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagReferenceMetadata {
    pub hash: H256,
    pub found: bool,
    pub level: u64,
}

/// Result of validating block level against pivot/tip metadata availability.
///
/// Output fields:
/// - `ok`: true only when there are no missing references and level matches.
/// - `expected_level`: max(parent-level + 1) across available pivot/tips.
/// - `level_matches`: whether `block_level == expected_level`.
/// - `missing_references`: missing pivot/tip hashes in deterministic order:
///   pivot first, then tips in provided order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagPivotTipsValidation {
    pub ok: bool,
    pub expected_level: u64,
    pub level_matches: bool,
    pub missing_references: Vec<H256>,
}

/// Maximum number of tips allowed on one DAG block.
///
/// This mirrors legacy `kDagBlockMaxTips` and is used by Rust verify prechecks
/// to preserve deterministic parity.
pub const DAG_BLOCK_MAX_TIPS: usize = 16;

/// Legacy C++ `DagManager::VerifyBlockReturnType::AheadBlock` value.
///
/// The Rust precheck returns legacy-compatible numeric codes because the CXX
/// bridge exposes plain structs, while the public C++ enum remains owned by the
/// existing DagManager API.
pub const DAG_VERIFY_REJECT_AHEAD_BLOCK: u32 = 2;

/// Legacy C++ `DagManager::VerifyBlockReturnType::FailedVdfVerification` value.
pub const DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION: u32 = 3;

/// Legacy C++ `DagManager::VerifyBlockReturnType::FutureBlock` value.
pub const DAG_VERIFY_REJECT_FUTURE_BLOCK: u32 = 4;

/// Legacy C++ `DagManager::VerifyBlockReturnType::NotEligible` value.
pub const DAG_VERIFY_REJECT_NOT_ELIGIBLE: u32 = 5;

/// Legacy C++ `DagManager::VerifyBlockReturnType::ExpiredBlock` value.
pub const DAG_VERIFY_REJECT_EXPIRED_BLOCK: u32 = 6;

/// Legacy C++ `DagManager::VerifyBlockReturnType::IncorrectTransactionsEstimation` value.
pub const DAG_VERIFY_REJECT_INCORRECT_TRANSACTIONS_ESTIMATION: u32 = 7;

/// Legacy C++ `DagManager::VerifyBlockReturnType::BlockTooBig` value.
pub const DAG_VERIFY_REJECT_BLOCK_TOO_BIG: u32 = 8;

/// Legacy C++ `DagManager::VerifyBlockReturnType::FailedTipsVerification` value.
pub const DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION: u32 = 9;

/// Legacy C++ `DagManager::VerifyBlockReturnType::MissingTip` value.
pub const DAG_VERIFY_REJECT_MISSING_TIP: u32 = 10;

/// Legacy C++ `DagManager::VerifyBlockReturnType::MissingTransaction` value.
pub const DAG_VERIFY_REJECT_MISSING_TRANSACTION: u32 = 1;

/// Rust DAG verification reason: continue validation.
pub const DAG_VERIFY_REASON_CONTINUE: u32 = 0;

/// Rust DAG verification reason: VRF key was not available.
pub const DAG_VERIFY_REASON_MISSING_VRF_KEY: u32 = 1;

/// Rust DAG verification reason: VDF proof did not validate.
pub const DAG_VERIFY_REASON_INVALID_VDF: u32 = 2;

/// Rust DAG verification reason: DPoS state for the block is not available.
pub const DAG_VERIFY_REASON_FUTURE_DPOS_SNAPSHOT: u32 = 3;

/// Rust DAG verification reason: block sender is not DPoS eligible.
pub const DAG_VERIFY_REASON_NOT_ELIGIBLE: u32 = 4;

/// VDF status: the VDF stage has not produced a fact yet.
pub const DAG_VERIFY_VDF_STATUS_NOT_CHECKED: u8 = 0;

/// VDF status: the VDF proof verified successfully.
pub const DAG_VERIFY_VDF_STATUS_VALID: u8 = 1;

/// VDF status: the VDF proof failed verification.
pub const DAG_VERIFY_VDF_STATUS_INVALID: u8 = 2;

/// DPoS status: the DPoS stage has not produced a fact yet.
pub const DAG_VERIFY_DPOS_STATUS_NOT_CHECKED: u8 = 0;

/// DPoS status: DPoS state for the proposal period is not available yet.
pub const DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE: u8 = 1;

/// DPoS status: the sender is eligible for the proposal period.
pub const DAG_VERIFY_DPOS_STATUS_ELIGIBLE: u8 = 2;

/// DPoS status: the sender is not eligible for the proposal period.
pub const DAG_VERIFY_DPOS_STATUS_NOT_ELIGIBLE: u8 = 3;

/// DAG proposer action: continue to the next local proposal stage.
pub const DAG_PROPOSER_ACTION_CONTINUE: u8 = 1;
/// DAG proposer action: skip this attempt and let the worker sleep.
pub const DAG_PROPOSER_ACTION_SKIP: u8 = 2;
/// DAG proposer action: retry later because an expected fact is not ready yet.
pub const DAG_PROPOSER_ACTION_RETRY_LATER: u8 = 3;

/// DAG proposer reason: all checks for the current stage passed.
pub const DAG_PROPOSER_REASON_OK: u32 = 0;
/// DAG proposer reason: no proposal-period mapping exists for the next DAG level.
pub const DAG_PROPOSER_REASON_MISSING_PROPOSAL_PERIOD: u32 = 1;
/// DAG proposer reason: the proposer has no VRF key in the DPoS snapshot.
pub const DAG_PROPOSER_REASON_MISSING_VRF_KEY: u32 = 2;
/// DAG proposer reason: wallet VRF public key does not match DPoS state.
pub const DAG_PROPOSER_REASON_VRF_KEY_MISMATCH: u32 = 3;
/// DAG proposer reason: DPoS state is unavailable for the proposal period.
pub const DAG_PROPOSER_REASON_DPOS_UNAVAILABLE: u32 = 4;
/// DAG proposer reason: proposer is not DPoS-eligible for the proposal period.
pub const DAG_PROPOSER_REASON_NOT_ELIGIBLE: u32 = 5;
/// DAG proposer reason: proposal vote denominator is zero.
pub const DAG_PROPOSER_REASON_ZERO_DENOMINATOR: u32 = 6;
/// DAG proposer reason: transaction pool is empty.
pub const DAG_PROPOSER_REASON_TRANSACTION_POOL_EMPTY: u32 = 7;
/// DAG proposer reason: non-finalized transaction count is above the proposal cap.
pub const DAG_PROPOSER_REASON_NON_FINALIZED_TRANSACTION_LIMIT: u32 = 8;
/// DAG proposer reason: FinalChain has not reached the proposal period yet.
pub const DAG_PROPOSER_REASON_FINALIZED_PERIOD_NOT_READY: u32 = 9;
/// DAG proposer reason: non-finalized DAG block count is above the hard pressure cap.
pub const DAG_PROPOSER_REASON_NON_FINALIZED_DAG_LIMIT: u32 = 10;
/// DAG proposer reason: low-difficulty non-finalized DAG pressure should delay the proposal.
pub const DAG_PROPOSER_REASON_LOW_DIFFICULTY_DAG_PRESSURE: u32 = 11;
/// DAG proposer reason: stale VDF difficulty should retry the same level later.
pub const DAG_PROPOSER_REASON_STALE_VDF_RETRY: u32 = 12;
/// DAG proposer reason: stale VDF difficulty started a new level retry window.
pub const DAG_PROPOSER_REASON_STALE_VDF_RESET: u32 = 13;
/// DAG proposer reason: live transaction packing selected no transactions.
pub const DAG_PROPOSER_REASON_PACKED_TRANSACTIONS_EMPTY: u32 = 14;

/// Inputs for deterministic `DagManager::verifyBlock` prechecks.
///
/// This struct covers only checks that do not need transaction bodies, VDF
/// execution, DPOS state, gas estimation, events, or network effects. It is
/// intentionally codec- and storage-independent so bridge/runtime code can
/// provide lookup results without moving infrastructure concerns into the
/// consensus domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagVerifyPrecheckInput {
    pub block_level: u64,
    pub pivot: H256,
    pub tips: Vec<H256>,
    pub proposal_period_found: bool,
    pub proposal_period: u64,
    pub dag_expiry_level: u64,
}

/// Decision returned by deterministic `DagManager::verifyBlock` prechecks.
///
/// `continue_validation == true` means only this Rust precheck passed; callers
/// must continue the remaining transaction, VDF, DPOS, and gas checks before
/// returning the public C++ `Verified` result. When `continue_validation` is
/// false, `reject_code` is one of the legacy-compatible reject constants in
/// this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagVerifyPrecheck {
    pub continue_validation: bool,
    pub reject_code: u32,
    pub proposal_period_found: bool,
    pub proposal_period: u64,
}

/// Per-tip gas metadata used by DAG block gas validation.
///
/// Missing tips are represented as data so consensus-invalid blocks return the
/// legacy `MissingTip` outcome instead of using error handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagTipGas {
    pub found: bool,
    pub gas_estimation: u64,
}

/// C++-materialized DAG tip metadata used by the Rust proposer planner.
///
/// Inputs:
/// - `hash`: candidate tip hash in frontier order.
/// - `found`: whether live DAG metadata was available for this candidate.
/// - `sender`, `level`, and `gas_estimation`: live block facts used only when
///   `found == true`.
///
/// Missing candidates are data, not errors, because legacy pruning skips them
/// only when tip selection is required. When no pruning is required the block
/// construction planner keeps frontier hashes in their original order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagProposerTipCandidate {
    pub hash: H256,
    pub found: bool,
    pub sender: [u8; 20],
    pub level: u64,
    pub gas_estimation: u64,
}

/// Deterministic tip-selection result for DAG proposal construction.
///
/// `selected` contains the hashes chosen for the proposed block. `skipped_missing`
/// counts missing candidates that were ignored during a pruning run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagProposerTipSelection {
    pub selected: Vec<H256>,
    pub skipped_missing: u64,
}

/// Inputs for Rust-owned DAG block construction planning.
///
/// The planner owns only deterministic policy: transaction gas summation,
/// deciding whether frontier tips must be pruned, and selecting tips when
/// pruning is required. C++ remains the temporary live object boundary for
/// transaction objects, VDF proof objects, and final `DagBlock` construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagProposerBlockConstructionInput {
    pub frontier_tips: Vec<DagProposerTipCandidate>,
    pub transaction_gas_estimations: Vec<u64>,
    pub pbft_gas_limit: u64,
    pub dag_gas_limit: u64,
    pub max_tips: u16,
}

/// Storage-backed inputs for Rust-owned DAG block construction planning.
///
/// C++ supplies only frontier-tip hashes, transaction gas estimates, gas limits, and the legacy max-tip limit. Rust
/// loads tip metadata from `rustaxa-storage` and recovers proposer senders from canonical DAG block RLP before applying
/// the deterministic block-construction planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagProposerStorageBlockConstructionInput {
    pub frontier_tips: Vec<H256>,
    pub transaction_gas_estimations: Vec<u64>,
    pub pbft_gas_limit: u64,
    pub dag_gas_limit: u64,
    pub max_tips: u16,
}

/// Rust DAG block construction plan consumed by the C++ proposer shim.
///
/// `block_gas_estimation` preserves legacy unsigned accumulation behavior with
/// wrapping addition. `pruned_tips` tells the shim whether the selected-tip list
/// came from the pruning policy or from the original frontier order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagProposerBlockConstructionPlan {
    pub selected_tips: Vec<H256>,
    pub block_gas_estimation: u64,
    pub pruned_tips: bool,
    pub skipped_missing_tips: u64,
}

/// Inputs for deterministic transaction availability checks in
/// `DagManager::verifyBlock`.
///
/// C++ owns live transaction lookup. Rust owns the deterministic decision over
/// expected and resolved transaction counts so missing-transaction semantics
/// stay explicit and testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagVerifyTransactionAvailabilityInput {
    pub expected_transactions: u64,
    pub resolved_transactions: u64,
}

/// Decision returned by deterministic transaction availability verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagVerifyTransactionAvailability {
    pub continue_validation: bool,
    pub reject_code: u32,
}

/// Transaction hash query plan for DAG manager C++ boundaries.
///
/// Rust owns deterministic hash selection while C++ still owns live
/// `Transaction` objects. The returned hashes preserve first-seen order so the
/// shim can query storage/pool without reimplementing ordering or dedup rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagTransactionQueryPlan {
    pub query_hashes: Vec<H256>,
}

/// Canonical DAG block payload selected for non-finalized sync.
///
/// `hash` is the requested DAG block hash and `block_rlp` is the exact
/// canonical payload loaded from `rustaxa-storage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagSyncBlockRlp {
    pub hash: H256,
    pub block_rlp: Vec<u8>,
}

/// Transaction payload lookup selected for non-finalized DAG sync.
///
/// `finalized` is true when the payload came from a finalized transaction
/// location instead of pending transaction storage. Missing hashes return
/// `found = false` and an empty payload so the bridge can preserve legacy packet
/// materialization behavior without deriving storage facts itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagTransactionStorageLookup {
    pub hash: H256,
    pub found: bool,
    pub finalized: bool,
    pub tx_rlp: Vec<u8>,
}

/// Storage-backed materialization payload for non-finalized DAG sync.
///
/// `blocks` preserves the selected DAG hash order. `transactions` contains
/// de-duplicated transaction payload lookups in first-seen block/transaction
/// order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagNonFinalizedSyncStoragePayload {
    pub blocks: Vec<DagSyncBlockRlp>,
    pub transactions: Vec<DagTransactionStorageLookup>,
}

/// Storage lookup result for a canonical DAG block payload.
///
/// Missing blocks are represented with `found = false` and an empty payload so
/// C++ compatibility callers can preserve their public optional-return shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagBlockStorageLookup {
    pub found: bool,
    pub block_rlp: Vec<u8>,
}

/// Storage lookup result for proposal-period rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagPeriodStorageLookup {
    pub found: bool,
    pub period: u64,
}

/// Storage lookup result for finalized DAG block period/position rows.
///
/// Missing rows are represented as `found = false` with zero values so C++
/// compatibility callers can preserve the legacy optional-return shape without
/// owning the storage lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagBlockPeriodStorageLookup {
    pub found: bool,
    pub period: u64,
    pub position: u32,
}

/// Storage lookup result for hash rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagHashStorageLookup {
    pub found: bool,
    pub hash: H256,
}

/// Persisted DAG block and edge counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagPersistenceCounters {
    pub dag_blocks: u64,
    pub dag_edges: u64,
}

/// Storage-backed precheck input for deterministic DAG block verification.
///
/// The proposal-period fact is intentionally not supplied by the bridge; this
/// helper reads it directly from `rustaxa-storage` before calling the pure
/// precheck planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagVerifyPrecheckStorageInput {
    pub block_level: u64,
    pub pivot: H256,
    pub tips: Vec<H256>,
    pub dag_expiry_level: u64,
}

/// Finalization fact for one transaction referenced by expired DAG blocks.
///
/// Inputs:
/// - `hash`: transaction hash candidate collected from an expired DAG block.
/// - `finalized`: true when storage already has a finalized location for the
///   transaction, in which case it must not be removed from non-finalized
///   transaction state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagExpiredTransactionFact {
    pub hash: H256,
    pub finalized: bool,
}

/// Transaction cleanup plan for expired DAG block finalization.
///
/// `remove_hashes` contains unique transaction hashes that were referenced by
/// expired DAG blocks, are not finalized, and are no longer referenced by any
/// remaining non-finalized DAG block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagExpiredTransactionCleanupPlan {
    pub remove_hashes: Vec<H256>,
}

/// Storage-backed expired DAG transaction cleanup payload.
///
/// `expired_transaction_facts` records the transaction references discovered in
/// expired DAG blocks together with finalized-index facts. `remove_hashes`
/// contains the planned non-finalized transaction rows to delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagExpiredTransactionCleanupStoragePayload {
    pub expired_transaction_facts: Vec<DagExpiredTransactionFact>,
    pub remove_hashes: Vec<H256>,
}

/// Storage facts required to update finalized DAG counters.
///
/// `hash` is the finalized DAG block hash, `level` is its persisted DAG level,
/// and `tips_count` is the persisted number of tips used for legacy edge-count
/// parity. These facts are loaded from `rustaxa-storage` so the bridge does not
/// derive counter writes from C++ storage state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagFinalizedCounterUpdate {
    pub hash: H256,
    pub level: u64,
    pub tips_count: u64,
}

/// Storage-backed cleanup payload for one finalized DAG order transition.
///
/// `counter_updates` are finalized counter/index facts loaded from storage,
/// `expired_hashes` are non-finalized DAG payload rows to delete, and
/// `remove_transaction_hashes` are non-finalized transaction payload rows to
/// delete after finalized/retained references have been considered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagManagerFinalizationCleanupStoragePayload {
    pub counter_updates: Vec<DagFinalizedCounterUpdate>,
    pub expired_hashes: Vec<H256>,
    pub remove_transaction_hashes: Vec<H256>,
}

/// Inputs for deterministic gas checks in `DagManager::verifyBlock`.
///
/// C++ still owns live transaction lookup and EVM-backed transaction gas
/// estimation. Rust owns the deterministic decision over the resulting counts,
/// weights, DAG/PBFT gas limits, and tip gas metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagVerifyGasInput {
    pub block_gas_estimation: u64,
    pub estimated_transactions_weight: u64,
    pub dag_gas_limit: u64,
    pub pbft_gas_limit: u64,
    pub tip_gas_estimations: Vec<DagTipGas>,
}

/// Decision returned by deterministic gas verification.
///
/// `continue_validation == true` means gas checks passed. When false,
/// `reject_code` is a legacy-compatible
/// `DagManager::VerifyBlockReturnType` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagVerifyGas {
    pub continue_validation: bool,
    pub reject_code: u32,
}

/// Inputs for preparing VDF verification in `DagManager::verifyBlock`.
///
/// C++ still owns live VRF-key lookup and DPoS vote-count/max-vote reads. Rust
/// owns the deterministic decision for missing VRF keys and carries the
/// supplied VDF vote counts to the remaining C++ VDF verifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagVerifyVdfPrepareInput {
    pub vrf_key_found: bool,
    pub eligible_vote_count: u64,
    pub vdf_max_vote_count: u64,
}

/// VDF verification preparation result.
///
/// When `continue_validation` is true, C++ must use `vote_count` and
/// `max_vote_count` for the C++ VDF verifier. When false, `reject_code` is a
/// legacy-compatible `VerifyBlockReturnType` value and `reason_code` explains
/// the Rust decision for tests and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagVerifyVdfPrepare {
    pub continue_validation: bool,
    pub reject_code: u32,
    pub reason_code: u32,
    pub vote_count: u64,
    pub max_vote_count: u64,
}

/// Inputs for deterministic authorization decisions in
/// `DagManager::verifyBlock`.
///
/// C++ still performs live VDF verification and DPoS state access. Rust owns
/// the ordering that maps those outcomes to legacy `VerifyBlockReturnType`
/// values. Missing VRF-key handling belongs to `prepare_dag_verify_vdf`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagVerifyAuthorizationInput {
    pub vdf_valid: bool,
    pub dpos_snapshot_available: bool,
    pub dpos_eligible: bool,
}

/// Decision returned by deterministic DAG block authorization verification.
///
/// `reason_code` is not a public C++ API value. It exists so bridge and Rust
/// tests can distinguish why one legacy reject code was selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagVerifyAuthorization {
    pub continue_validation: bool,
    pub reject_code: u32,
    pub reason_code: u32,
}

/// Staged VDF and DPoS fact envelope for `DagManager::verifyBlock`.
///
/// Inputs:
/// - `vrf_key_found`: whether the sender has a VRF key for the proposal period.
/// - `sender_eligible_vote_count`: sender vote count used by VDF sortition.
/// - `vdf_sortition_max_vote_count`: period-effective max vote count used by VDF sortition.
/// - `vdf_status`: one of the `DAG_VERIFY_VDF_STATUS_*` constants.
/// - `dpos_status`: one of the `DAG_VERIFY_DPOS_STATUS_*` constants.
///
/// This type is deliberately fact-only and supports staged collection. A
/// `*_NOT_CHECKED` status means that dependency has not produced a fact yet,
/// not that it succeeded. Infrastructure crates or C++ shims own live lookups
/// until those dependencies move behind Rust ports, while Rust owns the
/// deterministic reject ordering over the supplied facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagVerifyVdfDposFacts {
    pub vrf_key_found: bool,
    pub sender_eligible_vote_count: u64,
    pub vdf_sortition_max_vote_count: u64,
    pub vdf_status: u8,
    pub dpos_status: u8,
}

/// DPoS and VRF facts collected for DAG VDF authorization.
///
/// Inputs are collected from FinalChain state for one `(proposal_period,
/// sender)` pair. The optional `vrf_key` is included so the transitional C++
/// shim can continue running C++ VDF proof verification without repeating VRF
/// lookup through `KeyManager`.
///
/// Output invariants:
/// - `vrf_key_found` is true exactly when `vrf_key` contains a key.
/// - `sender_eligible_vote_count` and `vdf_sortition_max_vote_count` are the
///   vote values to pass to VDF sortition verification when a key exists and a
///   DPoS snapshot is available.
/// - `eligibility_status` is a `DAG_VERIFY_DPOS_STATUS_*` value and represents
///   missing snapshots as data rather than an infrastructure error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagDposAuthorizationFacts {
    pub vrf_key: Option<[u8; 32]>,
    pub vrf_key_found: bool,
    pub sender_eligible_vote_count: u64,
    pub vdf_sortition_max_vote_count: u64,
    pub eligibility_status: u8,
}

/// Rust-owned DAG proposer eligibility decision.
///
/// The action/reason pair uses the `DAG_PROPOSER_*` constants. Vote counts are populated when the proposer reaches a
/// denominator-related decision or can continue to VDF probing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagProposerEligibilityDecision {
    pub action: u8,
    pub reason_code: u32,
    pub vote_count: u64,
    pub max_vote_count: u64,
}

/// Input facts for the DAG proposer pre-VDF attempt planner.
///
/// This stage owns deterministic proposal gating before local VRF/VDF proof work starts. Infrastructure callers still
/// collect live pool sizes and FinalChain facts, but Rust decides whether the attempt should proceed to VDF preparation,
/// sleep, or retry later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagProposerPreVdfAttemptInput {
    pub transaction_pool_size: u64,
    pub non_finalized_transaction_count: u64,
    pub max_non_finalized_transactions: u64,
    pub proposal_period_found: bool,
    pub proposal_period: u64,
    pub proposal_level: u64,
    pub last_finalized_period: u64,
    pub dag_expiry_level_limit: u64,
    pub wallet_vrf_public_key: [u8; 32],
    pub authorization_facts: DagDposAuthorizationFacts,
}

/// Rust-owned DAG proposer pre-VDF attempt decision.
///
/// `action == DAG_PROPOSER_ACTION_CONTINUE` means C++ may compute the local VRF probe and VDF difficulty. `vote_count`
/// and `max_vote_count` are valid only for that continue case. `old_proposal` is a warning fact, not a reject reason,
/// preserving the legacy behavior that only logs old proposal attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagProposerPreVdfAttemptPlan {
    pub action: u8,
    pub reason_code: u32,
    pub proposal_period: u64,
    pub proposal_level: u64,
    pub last_finalized_period: u64,
    pub old_proposal: bool,
    pub vote_count: u64,
    pub max_vote_count: u64,
}

/// Input facts for the DAG proposer post-VDF attempt planner.
///
/// This stage owns deterministic proposal gating after local VRF probing has produced a VDF difficulty but before live
/// transaction materialization. C++ still performs the local cryptographic proof work and later transaction packing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagProposerPostVdfAttemptInput {
    pub frontier: DagProposerFrontierFacts,
    pub vdf_difficulty: u16,
    pub difficulty_min: u16,
    pub difficulty_stale: u16,
    pub max_non_finalized_dag_blocks: u64,
    pub max_non_finalized_dag_blocks_low_difficulty: u64,
    pub last_propose_level: u64,
    pub retry_count: u64,
    pub max_retry_count: u64,
    pub proposal_period: u64,
    pub proposal_weight_limit: u64,
    pub total_transaction_shards: u16,
    pub node_transaction_shard: u16,
    pub shard_period_interval: u64,
}

/// Transaction packing request emitted by the Rust DAG proposer attempt planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagProposerTransactionPackRequest {
    pub proposal_period: u64,
    pub weight_limit: u64,
    pub total_transaction_shards: u16,
    pub node_transaction_shard: u16,
    pub shard_period_interval: u64,
}

/// Rust-owned DAG proposer post-VDF attempt decision.
///
/// `action == DAG_PROPOSER_ACTION_CONTINUE` means C++ may request live transaction packing with `transaction_request`.
/// Retry state fields are authoritative when `retry_state_updated` is true and preserve legacy stale-difficulty retry
/// behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagProposerPostVdfAttemptPlan {
    pub action: u8,
    pub reason_code: u32,
    pub proposal_level: u64,
    pub vdf_stale: bool,
    pub retry_state_updated: bool,
    pub next_last_propose_level: u64,
    pub next_retry_count: u64,
    pub transaction_request: DagProposerTransactionPackRequest,
}

/// Input facts for the storage/runtime-backed DAG proposal attempt planner.
///
/// Rust owns deterministic proposal-attempt decisions and the local VRF probe needed to compute VDF difficulty. C++
/// still owns network throttling, live transaction packing/materialization, EVM gas estimation, async VDF proof over
/// selected transactions, final `DagBlock` construction, and network/add-block side effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagProposerAttemptInput {
    pub transaction_pool_size: u64,
    pub non_finalized_transaction_count: u64,
    pub max_non_finalized_transactions: u64,
    pub frontier: DagProposerFrontierFacts,
    pub proposal_period_found: bool,
    pub proposal_period: u64,
    pub last_finalized_period: u64,
    pub dag_expiry_level_limit: u64,
    pub period_block_hash_found: bool,
    pub period_block_hash: H256,
    pub wallet_vrf_public_key: [u8; 32],
    pub wallet_vrf_secret: [u8; 64],
    pub authorization_facts: DagDposAuthorizationFacts,
    pub sortition_params: crate::sortition::SortitionParams,
    pub max_non_finalized_dag_blocks: u64,
    pub max_non_finalized_dag_blocks_low_difficulty: u64,
    pub last_propose_level: u64,
    pub retry_count: u64,
    pub max_retry_count: u64,
    pub proposal_weight_limit: u64,
    pub total_transaction_shards: u16,
    pub node_transaction_shard: u16,
    pub shard_period_interval: u64,
}

/// Rust-owned DAG proposal attempt plan consumed by the C++ proposer shim.
///
/// `action == DAG_PROPOSER_ACTION_CONTINUE` means C++ may call live transaction packing with `transaction_request`.
/// Expected skips/retries are represented by `reason_code`; malformed crypto/sortition inputs are returned as errors by
/// [`plan_dag_proposer_attempt`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagProposerAttemptPlan {
    pub action: u8,
    pub reason_code: u32,
    pub frontier: DagFrontier,
    pub anchor: H256,
    pub proposal_level: u64,
    pub proposal_period_found: bool,
    pub proposal_period: u64,
    pub last_finalized_period: u64,
    pub period_block_hash_found: bool,
    pub period_block_hash: H256,
    pub vrf_input: Vec<u8>,
    pub vote_count: u64,
    pub max_vote_count: u64,
    pub vdf_difficulty: u16,
    pub vdf_stale: bool,
    pub old_proposal: bool,
    pub update_retry_state: bool,
    pub next_last_propose_level: u64,
    pub next_retry_count: u64,
    pub transaction_request: DagProposerTransactionPackRequest,
}

/// Input facts for the DAG proposer post-pack planner.
///
/// This stage runs after the live transaction manager boundary returns. Rust
/// owns the deterministic retry-state mutation for an empty packed result, but
/// it does not inspect transaction bodies, estimate gas, build VDF payloads, or
/// construct a `DagBlock`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagProposerPostPackInput {
    pub proposal_level: u64,
    pub packed_transaction_count: u64,
}

/// Rust-owned DAG proposer post-pack decision.
///
/// `action == DAG_PROPOSER_ACTION_CONTINUE` means C++ may continue to VDF proof
/// execution and block construction with the already-materialized transaction
/// list. Empty packed results return `SKIP` and carry the authoritative retry
/// state reset that legacy code performed locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagProposerPostPackPlan {
    pub action: u8,
    pub reason_code: u32,
    pub update_retry_state: bool,
    pub next_last_propose_level: u64,
    pub next_retry_count: u64,
}

/// Decision returned for the VDF and DPoS authorization stage.
///
/// Output invariants:
/// - missing VRF key and invalid VDF both map to legacy
///   `FailedVdfVerification`, distinguished by `reason_code`.
/// - unavailable DPoS state maps to `FutureBlock`.
/// - DPoS ineligibility maps to `NotEligible`.
/// - successful results pass through the exact vote counts supplied in the
///   input for diagnostics and compatibility checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagVerifyVdfDposDecision {
    pub continue_validation: bool,
    pub reject_code: u32,
    pub reason_code: u32,
    pub vote_count: u64,
    pub max_vote_count: u64,
}

/// Consensus-specific DAG VDF sortition verification input.
///
/// Inputs:
/// - `block_rlp`: canonical eight-field DAG block RLP, including transactions
///   and the embedded VDF sortition payload.
/// - `vdf_input`: canonical VDF message bytes used by the block proposer.
/// - `sortition_params`: runtime sortition parameters for the proposal period.
/// - `vrf_output`: legacy/compatibility precomputed VRF output. This field is
///   still required for legacy callers that have not migrated to embedded VRF
///   proof verification.
/// - `vrf_public_key` + `vrf_input`: optional Rust-owned embedded VRF contract.
///   When both are present, Rust verifies `vrf_proof` from the payload with
///   these inputs instead of trusting precomputed `vrf_output`.
/// - `sender_eligible_vote_count`: sender vote count used to normalize VRF
///   threshold selection.
/// - `vdf_sortition_max_vote_count`: period-effective max vote count used for
///   the vote normalization denominator.
///
/// Edge behavior:
/// malformed RLP and impossible runtime parameters are operational errors;
/// invalid difficulty or VDF proof are consensus-invalid facts returned as
/// `DAG_VERIFY_VDF_STATUS_INVALID`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagVdfSortitionInput {
    pub block_rlp: Vec<u8>,
    pub vdf_input: Vec<u8>,
    pub sortition_params: crate::sortition::SortitionParams,
    pub vrf_output: [u8; 64],
    pub vrf_public_key: Vec<u8>,
    pub vrf_input: Vec<u8>,
    pub sender_eligible_vote_count: u64,
    pub vdf_sortition_max_vote_count: u64,
}

/// Result of Rust-owned DAG VDF sortition verification.
///
/// Output invariants:
/// - `vdf_status` is `VALID` only when the embedded difficulty and Wesolowski
///   proof match the supplied sortition parameters and verified VRF output.
/// - `difficulty` is the embedded block difficulty decoded from the VDF RLP.
/// - `expected_difficulty` is the difficulty derived from the supplied VRF
///   output and vote counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagVdfSortitionVerification {
    pub vdf_status: u8,
    pub difficulty: u16,
    pub expected_difficulty: u16,
}

/// Decoded VDF sortition payload embedded in a DAG block.
///
/// The canonical C++ payload is `[vrf_proof, vdf_solution_proof,
/// vdf_solution_output, difficulty]`. The VRF proof is exposed separately so a
/// temporary shim boundary can keep using the legacy VRF verifier until the VRF
/// module is rewritten in Rust. The Wesolowski solution and difficulty are
/// consumed by Rust-owned DAG VDF verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagVdfSortitionPayload {
    pub vrf_proof: [u8; 80],
    pub vdf_solution_proof: Vec<u8>,
    pub vdf_solution_output: Vec<u8>,
    pub difficulty: u16,
}

/// Inputs for constructing and verifying DAG VDF data directly from block RLP.
///
/// `DagManager` verifies VDF sortition by:
/// - building a legacy VRF message from `(block_level, proposal_period_hash)`
/// - building a VDF message from `(pivot, tx_hashes)`
/// - validating both signatures/proofs against the embedded payload in `block_rlp`.
///
/// This keeps CXX payloads flat and explicit while preserving the legacy
/// message format in Rust, where canonical RLP bytes are recomputed from the
/// block data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagVdfSortitionBlockInput {
    pub block_rlp: Vec<u8>,
    pub block_level: u64,
    pub proposal_period_hash: H256,
    pub vrf_public_key: [u8; 32],
    pub sortition_params: crate::sortition::SortitionParams,
    pub sender_eligible_vote_count: u64,
    pub vdf_sortition_max_vote_count: u64,
}

/// Derives frontier from a ghost path and current leaves.
///
/// Behavior mirrors legacy DagManager frontier rules:
/// - empty ghost path returns `{ pivot: 0, tips: [] }`
/// - non-empty path sets `pivot` to the last ghost-path hash
/// - `tips` contains leaves except `pivot`
///
/// Additional deterministic guarantees:
/// - tip order is preserved from `leaves` while removing only `pivot`.
pub fn derive_frontier(ghost_path: &[H256], leaves: &[H256]) -> DagFrontier {
    let Some(pivot) = ghost_path.last().copied() else {
        return DagFrontier {
            pivot: H256::zero(),
            tips: Vec::new(),
        };
    };

    let tips = leaves
        .iter()
        .copied()
        .filter(|hash| *hash != pivot)
        .collect::<Vec<_>>();

    DagFrontier { pivot, tips }
}

/// Validates expected block level and missing pivot/tip references from metadata.
///
/// This mirrors legacy DagManager logic:
/// - `expected_level` starts at `0`
/// - each found pivot/tip updates `expected_level = max(expected_level, level + 1)`
///   with `u64` wrapping addition to mirror legacy C++ unsigned arithmetic
/// - missing references are returned for caller-driven sync requests
/// - final `ok` requires both no missing references and matching block level
pub fn validate_pivot_tips_metadata(
    block_level: u64,
    pivot: DagReferenceMetadata,
    tips: &[DagReferenceMetadata],
) -> DagPivotTipsValidation {
    let mut expected_level = 0_u64;
    let mut missing_references = Vec::new();

    if pivot.found {
        expected_level = expected_level.max(pivot.level.wrapping_add(1));
    } else {
        missing_references.push(pivot.hash);
    }

    for tip in tips {
        if tip.found {
            expected_level = expected_level.max(tip.level.wrapping_add(1));
        } else {
            missing_references.push(tip.hash);
        }
    }

    let level_matches = block_level == expected_level;
    let ok = missing_references.is_empty() && level_matches;

    DagPivotTipsValidation {
        ok,
        expected_level,
        level_matches,
        missing_references,
    }
}

/// Runs deterministic `DagManager::verifyBlock` prechecks.
///
/// The order mirrors the deterministic portion of the legacy C++ verification
/// path: tip count/uniqueness, proposal-period availability, and expiry. The
/// public `Verified` result is deliberately not produced here because successful
/// prechecks are only permission to continue the remaining verification stages.
pub fn validate_dag_verify_precheck(input: DagVerifyPrecheckInput) -> DagVerifyPrecheck {
    let reject_code = if input.tips.len() > DAG_BLOCK_MAX_TIPS {
        Some(DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION)
    } else {
        let mut unique_references = BTreeSet::from([input.pivot]);
        let has_duplicate_tip = input.tips.iter().any(|tip| !unique_references.insert(*tip));

        if has_duplicate_tip {
            Some(DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION)
        } else if !input.proposal_period_found {
            Some(DAG_VERIFY_REJECT_AHEAD_BLOCK)
        } else if input.block_level < input.dag_expiry_level {
            Some(DAG_VERIFY_REJECT_EXPIRED_BLOCK)
        } else {
            None
        }
    };

    DagVerifyPrecheck {
        continue_validation: reject_code.is_none(),
        reject_code: reject_code.unwrap_or(0),
        proposal_period_found: input.proposal_period_found,
        proposal_period: input.proposal_period,
    }
}

/// Runs deterministic transaction availability checks for
/// `DagManager::verifyBlock`.
///
/// The helper returns only `MissingTransaction` or continue; VDF/DPOS checks
/// still run before gas validation to preserve legacy return ordering.
pub fn validate_dag_verify_transaction_availability(
    input: DagVerifyTransactionAvailabilityInput,
) -> DagVerifyTransactionAvailability {
    let reject_code = (input.resolved_transactions < input.expected_transactions)
        .then_some(DAG_VERIFY_REJECT_MISSING_TRANSACTION);

    DagVerifyTransactionAvailability {
        continue_validation: reject_code.is_none(),
        reject_code: reject_code.unwrap_or(0),
    }
}

/// Plans which DAG block transaction hashes still need live lookup.
///
/// Inputs:
/// - `block_transaction_hashes`: transaction hashes in canonical block order.
/// - `supplied_transaction_hashes`: hashes already supplied by the caller, for
///   example sidecar transactions received with a DAG block.
///
/// Output:
/// - hashes from `block_transaction_hashes` missing from the supplied set,
///   preserving first-seen block order and deduplicating duplicate query
///   hashes. Callers that need duplicate transaction references should rebuild
///   those references from the original block hash list after lookup.
pub fn plan_dag_verify_transaction_query(
    block_transaction_hashes: &[H256],
    supplied_transaction_hashes: &[H256],
) -> DagTransactionQueryPlan {
    let supplied = supplied_transaction_hashes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut query_hashes = Vec::new();
    for hash in block_transaction_hashes {
        if supplied.contains(hash) {
            continue;
        }
        if seen.insert(*hash) {
            query_hashes.push(*hash);
        }
    }
    DagTransactionQueryPlan { query_hashes }
}

/// Plans unique transaction lookups for non-finalized DAG sync payloads.
///
/// Inputs:
/// - `block_transaction_hashes`: transaction hashes grouped in the same DAG
///   block order C++ will use for sync payload materialization.
///
/// Output:
/// - unique transaction hashes, preserving first-seen block/order position.
pub fn plan_non_finalized_transaction_query(
    block_transaction_hashes: &[Vec<H256>],
) -> DagTransactionQueryPlan {
    let mut seen = BTreeSet::new();
    let mut query_hashes = Vec::new();
    for block in block_transaction_hashes {
        for hash in block {
            if seen.insert(*hash) {
                query_hashes.push(*hash);
            }
        }
    }
    DagTransactionQueryPlan { query_hashes }
}

/// Collects non-finalized DAG sync payloads directly from Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `selected_hashes`: DAG block hashes selected by the Rust DAG state for
///   sync materialization.
///
/// Output:
/// - canonical block RLPs in selected hash order.
/// - unique transaction lookups in first-seen block/transaction order.
///
/// Edge behavior:
/// - A selected DAG block missing from storage is an explicit error; callers
///   must not emit a partial sync response from stale live state.
/// - Missing transactions are represented in the lookup payload rather than
///   treated as storage errors, matching the legacy sync packet surface.
pub fn collect_non_finalized_sync_payload_from_storage(
    storage: &Storage,
    selected_hashes: &[H256],
) -> Result<DagNonFinalizedSyncStoragePayload> {
    let mut transaction_hashes_by_block = Vec::with_capacity(selected_hashes.len());
    let mut blocks = Vec::with_capacity(selected_hashes.len());

    for hash in selected_hashes {
        let block_rlp = storage
            .dag()
            .by_hash_rlp_optional(*hash)
            .context("DAG_STORAGE_SYNC_BLOCK_LOAD")?
            .with_context(|| format!("DAG_STORAGE_SYNC_BLOCK_MISSING: {hash:?}"))?;

        let transactions = dag_block_transaction_hashes(&block_rlp)
            .context("DAG_STORAGE_SYNC_BLOCK_TRANSACTIONS")?;

        transaction_hashes_by_block.push(transactions);
        blocks.push(DagSyncBlockRlp {
            hash: *hash,
            block_rlp,
        });
    }

    let transaction_query = plan_non_finalized_transaction_query(&transaction_hashes_by_block);
    let transactions = transaction_storage_lookups(storage, &transaction_query.query_hashes)
        .context("DAG_STORAGE_SYNC_TRANSACTION_LOOKUP")?;

    Ok(DagNonFinalizedSyncStoragePayload {
        blocks,
        transactions,
    })
}

fn transaction_storage_lookups(
    storage: &Storage,
    hashes: &[H256],
) -> Result<Vec<DagTransactionStorageLookup>> {
    let transaction = storage.transaction();
    let mut out = Vec::with_capacity(hashes.len());

    for hash in hashes {
        let (tx_rlp, finalized) = if let Some(tx_rlp) = transaction
            .rlp(*hash)
            .context("DAG_STORAGE_TRANSACTION_RLP_PENDING_LOOKUP")?
        {
            (Some(tx_rlp), false)
        } else if let Some(location_rlp) = transaction
            .location_rlp(*hash)
            .context("DAG_STORAGE_TRANSACTION_RLP_LOCATION_LOOKUP")?
        {
            let location = Rlp::new(&location_rlp);
            let period = location
                .val_at::<u64>(0)
                .context("DAG_STORAGE_TRANSACTION_RLP_LOCATION_PERIOD")?;
            let position = location
                .val_at::<u32>(1)
                .context("DAG_STORAGE_TRANSACTION_RLP_LOCATION_POSITION")?;
            let is_system = location
                .item_count()
                .context("DAG_STORAGE_TRANSACTION_RLP_LOCATION_SHAPE")?
                == 3
                && location
                    .val_at::<bool>(2)
                    .context("DAG_STORAGE_TRANSACTION_RLP_LOCATION_SYSTEM_FLAG")?;
            let tx_rlp = if is_system {
                transaction
                    .system_rlp(*hash)
                    .context("DAG_STORAGE_TRANSACTION_RLP_SYSTEM_LOOKUP")?
            } else {
                transaction
                    .by_period_position_rlp(period, position)
                    .context("DAG_STORAGE_TRANSACTION_RLP_FINALIZED_LOOKUP")?
            };
            (tx_rlp, true)
        } else {
            (None, false)
        };

        out.push(DagTransactionStorageLookup {
            hash: *hash,
            found: tx_rlp.is_some(),
            finalized,
            tx_rlp: tx_rlp.unwrap_or_default(),
        });
    }

    Ok(out)
}

/// Returns whether Rust storage contains a DAG block in non-finalized or
/// finalized storage.
pub fn dag_block_exists_in_storage(storage: &Storage, hash: H256) -> Result<bool> {
    storage
        .dag()
        .exists(hash)
        .context("DAG_STORAGE_BLOCK_EXISTS")
}

/// Loads canonical DAG block RLP from Rust storage.
///
/// Missing rows are returned as `found = false` rather than errors so C++
/// compatibility accessors can preserve their optional lookup shape.
pub fn load_dag_block_from_storage(storage: &Storage, hash: H256) -> Result<DagBlockStorageLookup> {
    let block_rlp = storage
        .dag()
        .by_hash_rlp_optional(hash)
        .context("DAG_STORAGE_BLOCK_LOAD")?;
    Ok(match block_rlp {
        Some(block_rlp) => DagBlockStorageLookup {
            found: true,
            block_rlp,
        },
        None => DagBlockStorageLookup {
            found: false,
            block_rlp: Vec::new(),
        },
    })
}

/// Persists one non-finalized DAG block through Rust storage.
///
/// The storage module updates persistent DAG block/edge counters atomically with
/// the payload write. The consensus bridge supplies the canonical block hash,
/// level, tip count, and RLP after validation succeeds.
pub fn save_dag_block_to_storage(
    storage: &Storage,
    hash: H256,
    level: u64,
    tips_count: u64,
    block_rlp: &[u8],
) -> Result<()> {
    storage
        .dag()
        .write(hash, level, tips_count, block_rlp)
        .context("DAG_STORAGE_BLOCK_SAVE")
}

/// Ensures the proposal-period mapping exists for `level`.
///
/// Returns true when a mapping write was required and false when the existing
/// lookup already resolves to `period`.
pub fn ensure_proposal_period_mapping(storage: &Storage, level: u64, period: u64) -> Result<bool> {
    let dag = storage.dag();
    if dag
        .proposal_period_at_level(level)
        .context("DAG_PROPOSAL_PERIOD_LOOKUP")?
        == Some(period)
    {
        return Ok(false);
    }
    dag.write_proposal_period_at_level(level, period)
        .context("DAG_PROPOSAL_PERIOD_WRITE")?;
    Ok(true)
}

/// Resolves the finalized proposal period for a DAG level through Rust storage.
///
/// Rust storage returns the first persisted `(level -> period)` row at or after
/// the requested level. Missing rows are reported as `found = false`.
pub fn proposal_period_for_level_from_storage(
    storage: &Storage,
    level: u64,
) -> Result<DagPeriodStorageLookup> {
    let period = storage
        .dag()
        .proposal_period_at_level(level)
        .context("DAG_PROPOSAL_PERIOD_LOOKUP")?;
    Ok(match period {
        Some(period) => DagPeriodStorageLookup {
            found: true,
            period,
        },
        None => DagPeriodStorageLookup {
            found: false,
            period: 0,
        },
    })
}

/// Resolves a finalized DAG block's persisted PBFT period and position.
///
/// Inputs:
/// - `storage`: Rust storage handle owned by the calling consensus runtime.
/// - `hash`: canonical DAG block hash.
///
/// Outputs:
/// - Returns `found = true` with `(period, position)` when the finalized DAG
///   index contains `hash`, otherwise returns `found = false`.
///
/// Invariants and edge behavior:
/// - This is the Rust-owned equivalent of the read portion of
///   `DbStorage::getDagBlockPeriod` for consensus shims.
/// - Corrupt storage or backend failures are propagated as errors rather than
///   being converted to a missing row.
pub fn dag_block_period_from_storage(
    storage: &Storage,
    hash: H256,
) -> Result<DagBlockPeriodStorageLookup> {
    let lookup = storage
        .dag()
        .period_optional(hash)
        .context("DAG_BLOCK_PERIOD_LOOKUP")?;
    Ok(match lookup {
        Some((period, position)) => DagBlockPeriodStorageLookup {
            found: true,
            period,
            position,
        },
        None => DagBlockPeriodStorageLookup {
            found: false,
            period: 0,
            position: 0,
        },
    })
}

/// Returns the canonical PBFT block hash for finalized `period`.
///
/// The hash is derived from item 0 of the canonical `PeriodData` RLP stored in
/// Rust storage. Missing period data returns `found = false`; corrupt period
/// data is an error.
pub fn period_block_hash_from_storage(
    storage: &Storage,
    period: u64,
) -> Result<DagHashStorageLookup> {
    let period_data = storage
        .period()
        .data_raw(period)
        .context("DAG_PERIOD_DATA_LOOKUP")?;
    let Some(hash) = period_block_hash_from_period_data(&period_data)? else {
        return Ok(DagHashStorageLookup {
            found: false,
            hash: H256::zero(),
        });
    };
    Ok(DagHashStorageLookup { found: true, hash })
}

fn period_block_hash_from_period_data(period_data_rlp: &[u8]) -> Result<Option<H256>> {
    if period_data_rlp.is_empty() {
        return Ok(None);
    }

    let period_rlp = Rlp::new(period_data_rlp);
    let pbft_block_rlp = period_rlp
        .at(PBFT_BLOCK_POS_IN_PERIOD_DATA)
        .context("PERIOD_DATA_PBFT_BLOCK_RLP")?;
    let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(pbft_block_rlp.as_raw()))
        .context("PERIOD_DATA_PBFT_BLOCK_HASH")?;
    Ok(Some(link.block_hash))
}

/// Reads persisted DAG counters directly from Rust storage.
pub fn dag_persistence_counters_from_storage(storage: &Storage) -> Result<DagPersistenceCounters> {
    let metadata = storage.metadata();
    Ok(DagPersistenceCounters {
        dag_blocks: metadata
            .status_field(StatusField::DagBlkCount as u8)
            .context("DAG_STORAGE_COUNTERS")?,
        dag_edges: metadata
            .status_field(StatusField::DagEdgeCount as u8)
            .context("DAG_STORAGE_COUNTERS")?,
    })
}

/// Runs deterministic DAG block verification prechecks against Rust storage
/// facts and caller-supplied block metadata.
pub fn verify_precheck_from_storage(
    storage: &Storage,
    input: DagVerifyPrecheckStorageInput,
) -> Result<DagVerifyPrecheck> {
    let proposal_period = storage
        .dag()
        .proposal_period_at_level(input.block_level)
        .context("DAG_PROPOSAL_PERIOD_LOOKUP")?;
    Ok(validate_dag_verify_precheck(DagVerifyPrecheckInput {
        block_level: input.block_level,
        pivot: input.pivot,
        tips: input.tips,
        proposal_period_found: proposal_period.is_some(),
        proposal_period: proposal_period.unwrap_or(0),
        dag_expiry_level: input.dag_expiry_level,
    }))
}

/// Plans non-finalized transaction removals after expired DAG block cleanup.
///
/// Inputs:
/// - `expired_candidates`: transaction facts collected from expired DAG blocks
///   in deterministic block/transaction order. `finalized == true` marks
///   transaction candidates that must not be removed.
/// - `retained_transaction_refs`: transaction hashes still referenced by
///   remaining non-finalized DAG blocks.
///
/// Output:
/// - unique transaction hashes to remove, preserving first expired-candidate
///   order, excluding finalized transactions and retained references.
pub fn plan_expired_transaction_cleanup(
    expired_candidates: &[DagExpiredTransactionFact],
    retained_transaction_refs: &[H256],
) -> DagExpiredTransactionCleanupPlan {
    let finalized = expired_candidates
        .iter()
        .filter(|candidate| candidate.finalized)
        .map(|candidate| candidate.hash)
        .collect::<BTreeSet<_>>();
    let retained = retained_transaction_refs
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut remove_hashes = Vec::new();
    for candidate in expired_candidates {
        if finalized.contains(&candidate.hash) || retained.contains(&candidate.hash) {
            continue;
        }
        if seen.insert(candidate.hash) {
            remove_hashes.push(candidate.hash);
        }
    }
    DagExpiredTransactionCleanupPlan { remove_hashes }
}

/// Collects expired DAG transaction cleanup facts directly from Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `expired_hashes`: non-finalized DAG block hashes removed by a finalization
///   transition.
/// - `remaining_hashes`: non-finalized DAG block hashes retained after the
///   transition.
///
/// Outputs:
/// - Transaction facts from expired blocks in block/transaction order, with
///   finalized membership sourced from `rustaxa-storage`.
/// - The deterministic cleanup plan for non-finalized transaction rows that are
///   not finalized and no longer referenced by retained blocks.
///
/// Invariants and edge behavior:
/// - Missing expired or retained DAG block RLPs are explicit errors; callers
///   must not mutate live state when storage cannot supply the facts.
/// - Finalized transaction lookup is cached per hash for deterministic behavior
///   and to avoid repeated storage reads for duplicate transaction refs.
pub fn collect_expired_transaction_cleanup_from_storage(
    storage: &Storage,
    expired_hashes: &[H256],
    remaining_hashes: &[H256],
) -> Result<DagExpiredTransactionCleanupStoragePayload> {
    let mut finalized_cache = BTreeMap::new();
    let mut expired_candidates = Vec::new();
    let mut retained_transaction_hashes = Vec::new();

    for hash in expired_hashes {
        let block_rlp = storage
            .dag()
            .by_hash_rlp_optional(*hash)
            .context("DAG_STORAGE_EXPIRED_BLOCK_LOAD")?
            .with_context(|| format!("DAG_STORAGE_EXPIRED_BLOCK_MISSING: {hash:?}"))?;

        for trx_hash in dag_block_transaction_hashes(&block_rlp)
            .context("DAG_STORAGE_EXPIRED_BLOCK_TRANSACTIONS")?
        {
            let finalized = if let Some(finalized) = finalized_cache.get(&trx_hash) {
                *finalized
            } else {
                let finalized = storage
                    .transaction()
                    .finalized(trx_hash)
                    .context("DAG_STORAGE_TRANSACTION_FINALIZED")?;
                finalized_cache.insert(trx_hash, finalized);
                finalized
            };

            expired_candidates.push(DagExpiredTransactionFact {
                hash: trx_hash,
                finalized,
            });
        }
    }

    for hash in remaining_hashes {
        let block_rlp = storage
            .dag()
            .by_hash_rlp_optional(*hash)
            .context("DAG_STORAGE_REMAINING_BLOCK_LOAD")?
            .with_context(|| format!("DAG_STORAGE_REMAINING_BLOCK_MISSING: {hash:?}"))?;

        retained_transaction_hashes.extend(
            dag_block_transaction_hashes(&block_rlp)
                .context("DAG_STORAGE_REMAINING_BLOCK_TRANSACTIONS")?,
        );
    }

    let DagExpiredTransactionCleanupPlan { remove_hashes } =
        plan_expired_transaction_cleanup(&expired_candidates, &retained_transaction_hashes);

    Ok(DagExpiredTransactionCleanupStoragePayload {
        expired_transaction_facts: expired_candidates,
        remove_hashes,
    })
}

/// Collects finalized DAG cleanup facts directly from Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `counter_update_hashes`: finalized-order hashes that need persistent DAG
///   counter/index updates because they were not already non-finalized.
/// - `expired_hashes`: non-finalized DAG payload rows removed by the transition.
/// - `remaining_hashes`: non-finalized DAG payload rows retained after the
///   transition.
///
/// Output:
/// - counter update facts loaded from persisted DAG block metadata.
/// - expired DAG hashes preserved in caller order.
/// - non-finalized transaction hashes that are safe to delete after finalized
///   and retained references are filtered.
///
/// Edge behavior:
/// - Missing counter-update blocks, expired blocks, or retained blocks are
///   explicit errors.
/// - No storage writes are performed; use
///   `apply_finalization_cleanup_from_storage` to commit the resulting batch.
pub fn collect_finalization_cleanup_from_storage(
    storage: &Storage,
    counter_update_hashes: &[H256],
    expired_hashes: &[H256],
    remaining_hashes: &[H256],
) -> Result<DagManagerFinalizationCleanupStoragePayload> {
    let mut counter_updates = Vec::with_capacity(counter_update_hashes.len());
    for hash in counter_update_hashes {
        let block = storage
            .dag()
            .by_hash(*hash)
            .with_context(|| format!("DAG_STORAGE_FINALIZATION_COUNTER_BLOCK: {hash:?}"))?;
        counter_updates.push(DagFinalizedCounterUpdate {
            hash: *hash,
            level: block.level,
            tips_count: block.tips.len() as u64,
        });
    }

    let remove_transaction_hashes = if expired_hashes.is_empty() {
        Vec::new()
    } else {
        collect_expired_transaction_cleanup_from_storage(storage, expired_hashes, remaining_hashes)
            .context("DAG_STORAGE_FINALIZATION_TRANSACTION_CLEANUP")?
            .remove_hashes
    };

    Ok(DagManagerFinalizationCleanupStoragePayload {
        counter_updates,
        expired_hashes: expired_hashes.to_vec(),
        remove_transaction_hashes,
    })
}

/// Applies finalized DAG cleanup through one Rust-owned storage batch.
///
/// Inputs match `collect_finalization_cleanup_from_storage`.
///
/// Behavior:
/// - loads all cleanup facts from `rustaxa-storage`.
/// - commits counter/index updates, expired DAG deletes, and expired
///   non-finalized transaction deletes through `rustaxa-storage` in one batch.
/// - returns the committed cleanup payload so the C++ shim can perform
///   temporary live sidecar cleanup without deriving storage writes.
///
/// Edge behavior:
/// - If fact collection or the batch commit fails, no caller-owned live state
///   should be mutated.
pub fn apply_finalization_cleanup_from_storage(
    storage: &Storage,
    counter_update_hashes: &[H256],
    expired_hashes: &[H256],
    remaining_hashes: &[H256],
) -> Result<DagManagerFinalizationCleanupStoragePayload> {
    let payload = collect_finalization_cleanup_from_storage(
        storage,
        counter_update_hashes,
        expired_hashes,
        remaining_hashes,
    )?;
    let counter_updates = payload
        .counter_updates
        .iter()
        .map(|update| (update.hash, update.level, update.tips_count))
        .collect::<Vec<_>>();

    storage
        .dag()
        .apply_finalization_cleanup(
            &counter_updates,
            &payload.expired_hashes,
            &payload.remove_transaction_hashes,
        )
        .context("DAG_STORAGE_FINALIZATION_CLEANUP_APPLY")?;

    Ok(payload)
}

/// Prepares deterministic VDF inputs for `DagManager::verifyBlock`.
///
/// Missing VRF key is a consensus reject. On success, this returns the vote
/// count and max-vote count supplied by the current DPoS data source.
pub fn prepare_dag_verify_vdf(input: DagVerifyVdfPrepareInput) -> DagVerifyVdfPrepare {
    let reject_code = if !input.vrf_key_found {
        DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION
    } else {
        0
    };

    DagVerifyVdfPrepare {
        continue_validation: reject_code == 0,
        reject_code,
        reason_code: if reject_code == 0 {
            DAG_VERIFY_REASON_CONTINUE
        } else {
            DAG_VERIFY_REASON_MISSING_VRF_KEY
        },
        vote_count: input.eligible_vote_count,
        max_vote_count: input.vdf_max_vote_count,
    }
}

/// Runs deterministic gas checks for `DagManager::verifyBlock`.
///
/// This must be called after transaction availability, VDF, and DPOS checks to
/// preserve legacy `verifyBlock` return ordering. Tip count is derived from the
/// provided tip metadata so callers cannot accidentally bypass missing-tip or
/// aggregate-gas checks by passing inconsistent counts.
pub fn validate_dag_verify_gas(input: DagVerifyGasInput) -> DagVerifyGas {
    let reject_code = if input.block_gas_estimation > input.dag_gas_limit {
        Some(DAG_VERIFY_REJECT_BLOCK_TOO_BIG)
    } else if input.estimated_transactions_weight != input.block_gas_estimation {
        Some(DAG_VERIFY_REJECT_INCORRECT_TRANSACTIONS_ESTIMATION)
    } else if exceeds_pbft_dag_count(
        input.tip_gas_estimations.len() as u64,
        input.dag_gas_limit,
        input.pbft_gas_limit,
    ) {
        let mut total_gas = input.block_gas_estimation;
        for tip in input.tip_gas_estimations {
            if !tip.found {
                return DagVerifyGas {
                    continue_validation: false,
                    reject_code: DAG_VERIFY_REJECT_MISSING_TIP,
                };
            }
            total_gas = total_gas.wrapping_add(tip.gas_estimation);
        }
        (total_gas > input.pbft_gas_limit).then_some(DAG_VERIFY_REJECT_BLOCK_TOO_BIG)
    } else {
        None
    };

    DagVerifyGas {
        continue_validation: reject_code.is_none(),
        reject_code: reject_code.unwrap_or(0),
    }
}

/// Selects DAG proposer tips from caller-provided tip metadata.
///
/// The planner preserves legacy proposer policy:
/// - missing candidates are skipped and counted
/// - found candidates from unique proposers are considered before duplicate
///   proposer candidates
/// - each group is ordered by descending level with stable input-order ties
/// - selection stops before exceeding `gas_limit` or `max_tips`
pub fn plan_dag_proposer_tip_selection(
    candidates: Vec<DagProposerTipCandidate>,
    gas_limit: u64,
    max_tips: u16,
) -> DagProposerTipSelection {
    let skipped_missing = candidates
        .iter()
        .filter(|candidate| !candidate.found)
        .count() as u64;
    let found = candidates
        .into_iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.found)
        .collect::<Vec<_>>();

    let mut proposer_counts = BTreeMap::<[u8; 20], usize>::new();
    for (_, candidate) in &found {
        *proposer_counts.entry(candidate.sender).or_default() += 1;
    }

    let mut unique = Vec::new();
    let mut duplicate = Vec::new();
    for candidate in found {
        if proposer_counts
            .get(&candidate.1.sender)
            .copied()
            .unwrap_or_default()
            > 1
        {
            duplicate.push(candidate);
        } else {
            unique.push(candidate);
        }
    }

    unique.sort_by_key(|(position, candidate)| (Reverse(candidate.level), *position));
    duplicate.sort_by_key(|(position, candidate)| (Reverse(candidate.level), *position));

    let mut selected = Vec::new();
    let mut gas_used = 0_u64;
    for (_, candidate) in unique.into_iter().chain(duplicate) {
        gas_used = gas_used.saturating_add(candidate.gas_estimation);
        if gas_used > gas_limit || selected.len() == usize::from(max_tips) {
            break;
        }
        selected.push(candidate.hash);
    }

    DagProposerTipSelection {
        selected,
        skipped_missing,
    }
}

/// Plans deterministic DAG block construction facts for the proposer runtime.
///
/// C++ supplies live frontier-tip metadata and transaction gas estimates. Rust
/// owns the legacy gas summation and tip-pruning decision so the proposer shim
/// no longer decides these consensus facts around C++ storage/runtime objects.
/// `dag_gas_limit == 0` is treated as requiring tip pruning instead of
/// panicking on division by zero; valid chain configs never use zero here.
pub fn plan_dag_proposer_block_construction(
    input: DagProposerBlockConstructionInput,
) -> DagProposerBlockConstructionPlan {
    let block_gas_estimation = input
        .transaction_gas_estimations
        .iter()
        .fold(0_u64, |sum, gas| sum.wrapping_add(*gas));

    let frontier_tip_count = input.frontier_tips.len();
    let max_tips = usize::from(input.max_tips);
    let pbft_dag_tip_capacity = input
        .pbft_gas_limit
        .checked_div(input.dag_gas_limit)
        .unwrap_or(0);
    let pruned_tips = frontier_tip_count > max_tips
        || (frontier_tip_count as u64).saturating_add(1) > pbft_dag_tip_capacity;

    if pruned_tips {
        let selection = plan_dag_proposer_tip_selection(
            input.frontier_tips,
            input.pbft_gas_limit.wrapping_sub(block_gas_estimation),
            input.max_tips,
        );
        return DagProposerBlockConstructionPlan {
            selected_tips: selection.selected,
            block_gas_estimation,
            pruned_tips: true,
            skipped_missing_tips: selection.skipped_missing,
        };
    }

    DagProposerBlockConstructionPlan {
        selected_tips: input
            .frontier_tips
            .into_iter()
            .map(|candidate| candidate.hash)
            .collect(),
        block_gas_estimation,
        pruned_tips: false,
        skipped_missing_tips: 0,
    }
}

/// Plans DAG proposer block construction using tip metadata loaded from Rust storage.
///
/// Missing tip rows are represented as missing candidates and skipped by the underlying proposer-tip planner, preserving
/// the transitional C++ behavior where a null `DagBlock` tip is not selected during pruning. Malformed stored DAG RLP or
/// an unrecoverable stored tip signature returns an error because those indicate corrupted consensus storage rather than
/// a normal proposal decision.
pub fn plan_dag_proposer_block_construction_from_storage(
    storage: &Storage,
    input: DagProposerStorageBlockConstructionInput,
) -> Result<DagProposerBlockConstructionPlan> {
    let mut frontier_tips = Vec::with_capacity(input.frontier_tips.len());
    for hash in input.frontier_tips {
        let lookup = load_dag_block_from_storage(storage, hash)?;
        if !lookup.found {
            frontier_tips.push(DagProposerTipCandidate {
                hash,
                found: false,
                sender: [0; 20],
                level: 0,
                gas_estimation: 0,
            });
            continue;
        }

        let block = DagBlock::try_from(DagBlockRlp::new(&lookup.block_rlp))
            .with_context(|| format!("DAG_PROPOSER_TIP_RLP_DECODE: {hash:?}"))?;
        let sender = block
            .recover_sender()
            .with_context(|| format!("DAG_PROPOSER_TIP_SENDER_RECOVERY: {hash:?}"))?;
        frontier_tips.push(DagProposerTipCandidate {
            hash,
            found: true,
            sender: sender.0,
            level: block.level,
            gas_estimation: block.gas_estimation,
        });
    }

    Ok(plan_dag_proposer_block_construction(
        DagProposerBlockConstructionInput {
            frontier_tips,
            transaction_gas_estimations: input.transaction_gas_estimations,
            pbft_gas_limit: input.pbft_gas_limit,
            dag_gas_limit: input.dag_gas_limit,
            max_tips: input.max_tips,
        },
    ))
}

/// Plans one DAG proposal attempt up to the live transaction-packing boundary.
///
/// The planner performs no network work, no transaction materialization, no EVM gas estimation, and no async VDF proof.
/// It does own the deterministic pre-transaction proposal decisions, including pool/period/finalized-height readiness,
/// DPoS/VRF eligibility, local VRF probing for VDF difficulty, non-finalized DAG pressure, stale retry accounting, and
/// the transaction-packing request facts.
pub fn plan_dag_proposer_attempt(input: DagProposerAttemptInput) -> Result<DagProposerAttemptPlan> {
    let pre_plan = plan_dag_proposer_pre_vdf_attempt(DagProposerPreVdfAttemptInput {
        transaction_pool_size: input.transaction_pool_size,
        non_finalized_transaction_count: input.non_finalized_transaction_count,
        max_non_finalized_transactions: input.max_non_finalized_transactions,
        proposal_period_found: input.proposal_period_found,
        proposal_period: input.proposal_period,
        proposal_level: input.frontier.propose_level,
        last_finalized_period: input.last_finalized_period,
        dag_expiry_level_limit: input.dag_expiry_level_limit,
        wallet_vrf_public_key: input.wallet_vrf_public_key,
        authorization_facts: input.authorization_facts,
    });
    let mut plan = DagProposerAttemptPlan {
        action: pre_plan.action,
        reason_code: pre_plan.reason_code,
        frontier: input.frontier.frontier.clone(),
        anchor: input.frontier.anchor,
        proposal_level: input.frontier.propose_level,
        proposal_period_found: input.proposal_period_found,
        proposal_period: input.proposal_period,
        last_finalized_period: input.last_finalized_period,
        period_block_hash_found: input.period_block_hash_found,
        period_block_hash: input.period_block_hash,
        vrf_input: Vec::new(),
        vote_count: pre_plan.vote_count,
        max_vote_count: pre_plan.max_vote_count,
        vdf_difficulty: 0,
        vdf_stale: false,
        old_proposal: pre_plan.old_proposal,
        update_retry_state: false,
        next_last_propose_level: input.last_propose_level,
        next_retry_count: input.retry_count,
        transaction_request: DagProposerTransactionPackRequest {
            proposal_period: input.proposal_period,
            weight_limit: input.proposal_weight_limit,
            total_transaction_shards: input.total_transaction_shards,
            node_transaction_shard: input.node_transaction_shard,
            shard_period_interval: input.shard_period_interval,
        },
    };
    if pre_plan.action != DAG_PROPOSER_ACTION_CONTINUE {
        return Ok(plan);
    }

    let vrf_input = construct_dag_vrf_input(input.frontier.propose_level, input.period_block_hash);
    let normalized_vote_count =
        vdf_sortition::normalize_vote_count(pre_plan.vote_count, pre_plan.max_vote_count)?;
    let vrf_proof = vrf::prove(&input.wallet_vrf_secret, &vrf_input)?;
    let public_key = vrf::public_key_from_secret(&input.wallet_vrf_secret)?;
    let vrf_output = vrf::verify_output(&public_key, &vrf_proof, &vrf_input)?
        .ok_or_else(|| anyhow::anyhow!("Rust DAG proposer VRF proof did not verify"))?;
    let threshold = vrf::threshold_from_output(&vrf_output, normalized_vote_count);
    let vdf_difficulty = vdf_sortition::calculate_vdf_sortition_difficulty(
        vdf_sortition::VdfSortitionVerifyConfig {
            threshold_upper: input.sortition_params.vrf.threshold_upper,
            difficulty_min: input.sortition_params.vdf.difficulty_min,
            difficulty_max: input.sortition_params.vdf.difficulty_max,
            difficulty_stale: input.sortition_params.vdf.difficulty_stale,
            lambda_bound: input.sortition_params.vdf.lambda_bound,
        },
        threshold,
    )?;

    let post_plan = plan_dag_proposer_post_vdf_attempt(DagProposerPostVdfAttemptInput {
        frontier: input.frontier,
        vdf_difficulty,
        difficulty_min: input.sortition_params.vdf.difficulty_min,
        difficulty_stale: input.sortition_params.vdf.difficulty_stale,
        max_non_finalized_dag_blocks: input.max_non_finalized_dag_blocks,
        max_non_finalized_dag_blocks_low_difficulty: input
            .max_non_finalized_dag_blocks_low_difficulty,
        last_propose_level: input.last_propose_level,
        retry_count: input.retry_count,
        max_retry_count: input.max_retry_count,
        proposal_period: input.proposal_period,
        proposal_weight_limit: input.proposal_weight_limit,
        total_transaction_shards: input.total_transaction_shards,
        node_transaction_shard: input.node_transaction_shard,
        shard_period_interval: input.shard_period_interval,
    });
    plan.action = post_plan.action;
    plan.reason_code = post_plan.reason_code;
    plan.vrf_input = vrf_input;
    plan.vdf_difficulty = vdf_difficulty;
    plan.vdf_stale = post_plan.vdf_stale;
    plan.update_retry_state = post_plan.retry_state_updated;
    plan.next_last_propose_level = post_plan.next_last_propose_level;
    plan.next_retry_count = post_plan.next_retry_count;
    plan.transaction_request = post_plan.transaction_request;
    Ok(plan)
}

/// Plans the deterministic DAG proposer action after live transaction packing.
///
/// C++ still owns the live transaction-pool read, materialization, and EVM gas
/// estimation boundary. Rust owns only the protocol decision for the observed
/// packed count so retry state cannot silently diverge from the Rust proposer
/// runtime.
pub fn plan_dag_proposer_post_pack(input: DagProposerPostPackInput) -> DagProposerPostPackPlan {
    if input.packed_transaction_count == 0 {
        return DagProposerPostPackPlan {
            action: DAG_PROPOSER_ACTION_SKIP,
            reason_code: DAG_PROPOSER_REASON_PACKED_TRANSACTIONS_EMPTY,
            update_retry_state: true,
            next_last_propose_level: input.proposal_level,
            next_retry_count: 0,
        };
    }

    DagProposerPostPackPlan {
        action: DAG_PROPOSER_ACTION_CONTINUE,
        reason_code: DAG_PROPOSER_REASON_OK,
        update_retry_state: false,
        next_last_propose_level: input.proposal_level,
        next_retry_count: 0,
    }
}

/// Plans the DAG proposer attempt up to local VRF/VDF probing.
///
/// Expected unavailable facts are returned as status decisions, not errors. This preserves the proposer loop behavior
/// while moving proposal-period readiness, pool pressure, finalized-height readiness, and DPoS/VRF eligibility ordering
/// into Rust.
pub fn plan_dag_proposer_pre_vdf_attempt(
    input: DagProposerPreVdfAttemptInput,
) -> DagProposerPreVdfAttemptPlan {
    let old_proposal = input
        .proposal_period
        .saturating_add(input.dag_expiry_level_limit)
        < input.last_finalized_period;
    let mut plan = DagProposerPreVdfAttemptPlan {
        action: DAG_PROPOSER_ACTION_SKIP,
        reason_code: DAG_PROPOSER_REASON_OK,
        proposal_period: input.proposal_period,
        proposal_level: input.proposal_level,
        last_finalized_period: input.last_finalized_period,
        old_proposal,
        vote_count: 0,
        max_vote_count: 0,
    };

    if input.transaction_pool_size == 0 {
        plan.reason_code = DAG_PROPOSER_REASON_TRANSACTION_POOL_EMPTY;
        return plan;
    }
    if input.non_finalized_transaction_count > input.max_non_finalized_transactions {
        plan.reason_code = DAG_PROPOSER_REASON_NON_FINALIZED_TRANSACTION_LIMIT;
        return plan;
    }
    if !input.proposal_period_found {
        plan.reason_code = DAG_PROPOSER_REASON_MISSING_PROPOSAL_PERIOD;
        return plan;
    }
    if input.last_finalized_period < input.proposal_period {
        plan.action = DAG_PROPOSER_ACTION_RETRY_LATER;
        plan.reason_code = DAG_PROPOSER_REASON_FINALIZED_PERIOD_NOT_READY;
        return plan;
    }

    let eligibility =
        plan_dag_proposer_eligibility(input.wallet_vrf_public_key, input.authorization_facts);
    if eligibility.action != DAG_PROPOSER_ACTION_CONTINUE {
        plan.action = eligibility.action;
        plan.reason_code = eligibility.reason_code;
        plan.vote_count = eligibility.vote_count;
        plan.max_vote_count = eligibility.max_vote_count;
        return plan;
    }

    plan.action = DAG_PROPOSER_ACTION_CONTINUE;
    plan.reason_code = DAG_PROPOSER_REASON_OK;
    plan.vote_count = eligibility.vote_count;
    plan.max_vote_count = eligibility.max_vote_count;
    plan
}

/// Plans the DAG proposer attempt after local VRF probing and before transaction packing.
///
/// Rust owns non-finalized DAG pressure and stale-difficulty retry decisions. C++ must apply the returned retry state
/// when `retry_state_updated` is true and may request transaction packing only when `action == CONTINUE`.
pub fn plan_dag_proposer_post_vdf_attempt(
    input: DagProposerPostVdfAttemptInput,
) -> DagProposerPostVdfAttemptPlan {
    let transaction_request = DagProposerTransactionPackRequest {
        proposal_period: input.proposal_period,
        weight_limit: input.proposal_weight_limit,
        total_transaction_shards: input.total_transaction_shards,
        node_transaction_shard: input.node_transaction_shard,
        shard_period_interval: input.shard_period_interval,
    };
    let mut plan = DagProposerPostVdfAttemptPlan {
        action: DAG_PROPOSER_ACTION_SKIP,
        reason_code: DAG_PROPOSER_REASON_OK,
        proposal_level: input.frontier.propose_level,
        vdf_stale: input.vdf_difficulty == input.difficulty_stale,
        retry_state_updated: false,
        next_last_propose_level: input.last_propose_level,
        next_retry_count: input.retry_count,
        transaction_request,
    };

    if input.frontier.frontier.pivot != input.frontier.anchor {
        if input.frontier.non_finalized_block_count as u64 > input.max_non_finalized_dag_blocks {
            plan.reason_code = DAG_PROPOSER_REASON_NON_FINALIZED_DAG_LIMIT;
            return plan;
        }
        if input.frontier.non_finalized_min_difficulty < u32::from(input.vdf_difficulty)
            && input.frontier.non_finalized_block_count as u64
                > input.max_non_finalized_dag_blocks_low_difficulty
        {
            plan.reason_code = DAG_PROPOSER_REASON_LOW_DIFFICULTY_DAG_PRESSURE;
            return plan;
        }
    }

    if plan.vdf_stale {
        if input.last_propose_level == input.frontier.propose_level {
            if input.retry_count < input.max_retry_count {
                plan.reason_code = DAG_PROPOSER_REASON_STALE_VDF_RETRY;
                plan.retry_state_updated = true;
                plan.next_retry_count = input.retry_count.saturating_add(1);
                return plan;
            }
        } else {
            plan.reason_code = DAG_PROPOSER_REASON_STALE_VDF_RESET;
            plan.retry_state_updated = true;
            plan.next_last_propose_level = input.frontier.propose_level;
            plan.next_retry_count = 0;
            return plan;
        }
    }

    plan.action = DAG_PROPOSER_ACTION_CONTINUE;
    plan.reason_code = DAG_PROPOSER_REASON_OK;
    plan
}

fn plan_dag_proposer_eligibility(
    wallet_vrf_public_key: [u8; 32],
    authorization_facts: DagDposAuthorizationFacts,
) -> DagProposerEligibilityDecision {
    if !authorization_facts.vrf_key_found {
        return dag_proposer_decision(
            DAG_PROPOSER_ACTION_SKIP,
            DAG_PROPOSER_REASON_MISSING_VRF_KEY,
            0,
            0,
        );
    }
    if authorization_facts.vrf_key != Some(wallet_vrf_public_key) {
        return dag_proposer_decision(
            DAG_PROPOSER_ACTION_SKIP,
            DAG_PROPOSER_REASON_VRF_KEY_MISMATCH,
            0,
            0,
        );
    }
    if authorization_facts.eligibility_status == DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE {
        return dag_proposer_decision(
            DAG_PROPOSER_ACTION_RETRY_LATER,
            DAG_PROPOSER_REASON_DPOS_UNAVAILABLE,
            0,
            0,
        );
    }
    if authorization_facts.eligibility_status != DAG_VERIFY_DPOS_STATUS_ELIGIBLE {
        return dag_proposer_decision(
            DAG_PROPOSER_ACTION_SKIP,
            DAG_PROPOSER_REASON_NOT_ELIGIBLE,
            0,
            0,
        );
    }
    if authorization_facts.vdf_sortition_max_vote_count == 0 {
        return dag_proposer_decision(
            DAG_PROPOSER_ACTION_SKIP,
            DAG_PROPOSER_REASON_ZERO_DENOMINATOR,
            authorization_facts.sender_eligible_vote_count,
            0,
        );
    }

    dag_proposer_decision(
        DAG_PROPOSER_ACTION_CONTINUE,
        DAG_PROPOSER_REASON_OK,
        authorization_facts.sender_eligible_vote_count,
        authorization_facts.vdf_sortition_max_vote_count,
    )
}

fn dag_proposer_decision(
    action: u8,
    reason_code: u32,
    vote_count: u64,
    max_vote_count: u64,
) -> DagProposerEligibilityDecision {
    DagProposerEligibilityDecision {
        action,
        reason_code,
        vote_count,
        max_vote_count,
    }
}

/// Runs deterministic authorization checks for `DagManager::verifyBlock`.
///
/// This must be called after transaction availability and before gas checks to
/// preserve legacy return ordering. Invalid VDF proof maps to
/// `FailedVdfVerification`; DPoS state unavailability maps to `FutureBlock`;
/// ineligible validators map to `NotEligible`.
pub fn validate_dag_verify_authorization(
    input: DagVerifyAuthorizationInput,
) -> DagVerifyAuthorization {
    let (reject_code, reason_code) = if !input.vdf_valid {
        (
            DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION,
            DAG_VERIFY_REASON_INVALID_VDF,
        )
    } else if !input.dpos_snapshot_available {
        (
            DAG_VERIFY_REJECT_FUTURE_BLOCK,
            DAG_VERIFY_REASON_FUTURE_DPOS_SNAPSHOT,
        )
    } else if !input.dpos_eligible {
        (
            DAG_VERIFY_REJECT_NOT_ELIGIBLE,
            DAG_VERIFY_REASON_NOT_ELIGIBLE,
        )
    } else {
        (0, DAG_VERIFY_REASON_CONTINUE)
    };

    DagVerifyAuthorization {
        continue_validation: reject_code == 0,
        reject_code,
        reason_code,
    }
}

/// Runs the staged deterministic VDF and DPoS authorization decision.
///
/// The input contains facts gathered by the current runtime boundary. This
/// function centralizes consensus reject ordering so shims do not encode that
/// policy while VDF, FinalChain, and DPoS data access are still migrating.
pub fn decide_dag_verify_vdf_dpos_authorization(
    facts: DagVerifyVdfDposFacts,
) -> DagVerifyVdfDposDecision {
    let (reject_code, reason_code) = if !facts.vrf_key_found {
        (
            DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION,
            DAG_VERIFY_REASON_MISSING_VRF_KEY,
        )
    } else if facts.vdf_status == DAG_VERIFY_VDF_STATUS_INVALID {
        (
            DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION,
            DAG_VERIFY_REASON_INVALID_VDF,
        )
    } else if facts.dpos_status == DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE {
        (
            DAG_VERIFY_REJECT_FUTURE_BLOCK,
            DAG_VERIFY_REASON_FUTURE_DPOS_SNAPSHOT,
        )
    } else if facts.dpos_status == DAG_VERIFY_DPOS_STATUS_NOT_ELIGIBLE {
        (
            DAG_VERIFY_REJECT_NOT_ELIGIBLE,
            DAG_VERIFY_REASON_NOT_ELIGIBLE,
        )
    } else {
        (0, DAG_VERIFY_REASON_CONTINUE)
    };

    DagVerifyVdfDposDecision {
        continue_validation: reject_code == 0,
        reject_code,
        reason_code,
        vote_count: facts.sender_eligible_vote_count,
        max_vote_count: facts.vdf_sortition_max_vote_count,
    }
}

/// Extracts the VRF proof embedded in canonical DAG block RLP.
///
/// This helper remains available for legacy compatibility paths that still need
/// to extract the embedded VRF proof from the payload. Malformed block or VDF
/// RLP is returned as an operational bridge error because callers cannot verify
/// the peer-supplied payload without decoding it.
pub fn extract_dag_vdf_vrf_proof(block_rlp: &[u8]) -> Result<[u8; 80]> {
    Ok(decode_dag_vdf_sortition_payload(block_rlp)?.vrf_proof)
}

/// Verifies the DAG block VDF proof and sortition difficulty in Rust.
///
/// The preferred input is an embedded VRF proof input tuple:
/// `vrf_public_key` + `vrf_input`. When both fields are supplied, Rust verifies
/// the embedded proof directly from the DAG block payload.
/// Legacy callers can still provide `vrf_output` for compatibility while the
/// migration to embedded verification completes.
pub fn verify_dag_vdf_sortition(
    input: DagVdfSortitionInput,
) -> Result<DagVdfSortitionVerification> {
    let block = DagBlock::try_from(DagBlockRlp::new(&input.block_rlp))
        .context("decode canonical DAG block RLP for VDF verification")?;
    let payload = decode_vdf_sortition_payload(&block.vdf)
        .context("decode DAG block VDF sortition payload")?;

    let verify_embedded_vrf = !(input.vrf_public_key.is_empty() && input.vrf_input.is_empty());
    ensure!(
        !(verify_embedded_vrf && (input.vrf_public_key.is_empty() || input.vrf_input.is_empty())),
        "embedded VRF verification requires both vrf_public_key and vrf_input"
    );

    if verify_embedded_vrf {
        ensure!(
            input.vrf_public_key.len() == 32,
            "embedded VRF public key must be 32 bytes"
        );
        let mut public_key = [0_u8; 32];
        public_key.copy_from_slice(&input.vrf_public_key);
        let result = sortition::verify_legacy_vdf_sortition(
            LegacySortitionParams {
                vrf_threshold_upper: input.sortition_params.vrf.threshold_upper,
                vdf_difficulty_min: input.sortition_params.vdf.difficulty_min,
                vdf_difficulty_max: input.sortition_params.vdf.difficulty_max,
                vdf_difficulty_stale: input.sortition_params.vdf.difficulty_stale,
                vdf_lambda_bound: input.sortition_params.vdf.lambda_bound,
            },
            &public_key,
            &block.vdf,
            &input.vrf_input,
            &input.vdf_input,
            input.sender_eligible_vote_count,
            input.vdf_sortition_max_vote_count,
        )?;

        let vdf_status = if result.status == sortition::LEGACY_SORTITION_STATUS_VALID {
            DAG_VERIFY_VDF_STATUS_VALID
        } else {
            DAG_VERIFY_VDF_STATUS_INVALID
        };

        return Ok(DagVdfSortitionVerification {
            vdf_status,
            difficulty: result.actual_difficulty,
            expected_difficulty: result.expected_difficulty,
        });
    }

    let normalized_vote_count = normalized_vdf_vote_count(
        input.sender_eligible_vote_count,
        input.vdf_sortition_max_vote_count,
    )?;
    let threshold = threshold_from_vrf_output(&input.vrf_output, normalized_vote_count);
    let expected_difficulty =
        calculate_vdf_sortition_difficulty(input.sortition_params, threshold)?;

    if payload.difficulty != expected_difficulty {
        return Ok(DagVdfSortitionVerification {
            vdf_status: DAG_VERIFY_VDF_STATUS_INVALID,
            difficulty: payload.difficulty,
            expected_difficulty,
        });
    }

    let solution = VdfSolution {
        first: payload.vdf_solution_proof,
        second: payload.vdf_solution_output,
    };
    let vdf = WesolowskiVdf::new(
        u32::from(input.sortition_params.vdf.lambda_bound),
        u32::from(payload.difficulty),
        input.vdf_input,
        LEGACY_VDF_MODULUS_ASCII_HEX.to_vec(),
    );
    let verifier = WesolowskiVerifier::new(&vdf);
    let vdf_status = if verifier.verify(&solution) {
        DAG_VERIFY_VDF_STATUS_VALID
    } else {
        DAG_VERIFY_VDF_STATUS_INVALID
    };

    Ok(DagVdfSortitionVerification {
        vdf_status,
        difficulty: payload.difficulty,
        expected_difficulty,
    })
}

/// Builds the legacy VRF input message for DAG sortition.
///
/// The C++ `VrfSortitionBase::makeVrfInput` uses a default `dev::RLPStream`
/// and appends `block_level` followed by `proposal_period_hash`. This is a
/// sequence of two RLP items, not an RLP list.
pub fn construct_dag_vrf_input(block_level: u64, proposal_period_hash: H256) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new();
    stream.append(&block_level);
    stream.append(&proposal_period_hash);
    stream.out().to_vec()
}

/// Builds the legacy VDF message from DAG block pivot and transaction hashes.
///
/// The C++ `DagManager::getVdfMessage` uses a default `dev::RLPStream` and
/// appends the pivot followed by each transaction hash. This is a sequence of
/// RLP items, not an RLP list.
pub fn construct_dag_vdf_message(pivot: H256, transaction_hashes: &[H256]) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new();
    stream.append(&pivot);
    for tx_hash in transaction_hashes {
        stream.append(tx_hash);
    }
    stream.out().to_vec()
}

/// Extracts transaction hashes from a canonical DAG block payload.
///
/// Inputs:
/// - `block_rlp`: canonical eight-field DAG block bytes.
///
/// Output is the block transaction list in canonical order, preserved for
/// downstream sync and validation logic.
pub fn dag_block_transaction_hashes(block_rlp: &[u8]) -> Result<Vec<H256>> {
    let block = DagBlock::try_from(DagBlockRlp::new(block_rlp))
        .context("decode canonical DAG block RLP for transaction hash extraction")?;
    Ok(block.transactions)
}

/// Builds the legacy VDF message from canonical DAG block RLP.
///
/// `block_rlp` must be the canonical eight-field DAG block payload. The pivot
/// and transaction hashes are decoded and then encoded through
/// `construct_dag_vdf_message`.
pub fn construct_dag_vdf_message_from_block_rlp(block_rlp: &[u8]) -> Result<Vec<u8>> {
    let block = DagBlock::try_from(DagBlockRlp::new(block_rlp))
        .context("decode canonical DAG block RLP for VDF message construction")?;
    Ok(construct_dag_vdf_message(block.pivot, &block.transactions))
}

/// Verifies DAG VDF sortition from canonical block payload and high-level sortition
/// parameters.
///
/// Rust re-builds both:
/// - VRF message from `block_level` + `proposal_period_hash`
/// - VDF message from `pivot` + `transactions` of `block_rlp`
///
/// This path requires explicit `vrf_public_key` and thus always uses embedded
/// VRF verification. Legacy precomputed-output compatibility should route through
/// `verify_dag_vdf_sortition` directly.
pub fn verify_dag_vdf_sortition_from_block(
    input: DagVdfSortitionBlockInput,
) -> Result<DagVdfSortitionVerification> {
    let block = DagBlock::try_from(DagBlockRlp::new(&input.block_rlp))
        .context("decode canonical DAG block RLP for VDF verification")?;
    ensure!(
        block.level == input.block_level,
        "block level mismatch: input={} block={}",
        input.block_level,
        block.level
    );

    let vrf_input = construct_dag_vrf_input(input.block_level, input.proposal_period_hash);
    let vdf_input = construct_dag_vdf_message(block.pivot, &block.transactions);

    verify_dag_vdf_sortition(DagVdfSortitionInput {
        block_rlp: input.block_rlp,
        vdf_input,
        sortition_params: input.sortition_params,
        vrf_output: [0_u8; 64],
        vrf_public_key: input.vrf_public_key.to_vec(),
        vrf_input,
        sender_eligible_vote_count: input.sender_eligible_vote_count,
        vdf_sortition_max_vote_count: input.vdf_sortition_max_vote_count,
    })
}

fn decode_dag_vdf_sortition_payload(block_rlp: &[u8]) -> Result<DagVdfSortitionPayload> {
    let block = DagBlock::try_from(DagBlockRlp::new(block_rlp))
        .context("decode canonical DAG block RLP for VDF payload")?;
    decode_vdf_sortition_payload(&block.vdf)
}

fn decode_vdf_sortition_payload(vdf_rlp: &[u8]) -> Result<DagVdfSortitionPayload> {
    const VDF_SORTITION_FIELD_COUNT: usize = 4;
    const VRF_PROOF_BYTES: usize = 80;

    let rlp = Rlp::new(vdf_rlp);
    ensure!(
        rlp.item_count()? == VDF_SORTITION_FIELD_COUNT,
        "invalid DAG VDF sortition field count"
    );

    let proof_bytes = rlp.at(0)?.data()?;
    ensure!(
        proof_bytes.len() == VRF_PROOF_BYTES,
        "invalid DAG VDF VRF proof length"
    );
    let mut vrf_proof = [0_u8; VRF_PROOF_BYTES];
    vrf_proof.copy_from_slice(proof_bytes);

    Ok(DagVdfSortitionPayload {
        vrf_proof,
        vdf_solution_proof: rlp.val_at(1)?,
        vdf_solution_output: rlp.val_at(2)?,
        difficulty: rlp.val_at(3)?,
    })
}

fn normalized_vdf_vote_count(vote_count: u64, total_vote_count: u64) -> Result<u16> {
    const VOTES_PROPORTION: u64 = 1000;

    ensure!(
        total_vote_count != 0,
        "VDF sortition max vote count cannot be zero"
    );
    let normalized = vote_count
        .checked_mul(VOTES_PROPORTION)
        .context("VDF sortition vote normalization overflow")?
        / total_vote_count;
    ensure!(
        normalized <= u64::from(u16::MAX),
        "VDF sortition normalized vote count exceeds u16"
    );
    Ok(normalized as u16)
}

fn threshold_from_vrf_output(vrf_output: &[u8; 64], vote_count: u16) -> u16 {
    const MINSTD_RAND_MULTIPLIER: u16 = 48271;

    let mut threshold = (u16::from(vrf_output[1]) << 8) | u16::from(vrf_output[0]);
    if vote_count > 1 {
        let mut min_threshold = threshold;
        let mut threshold_candidate = threshold;
        for _ in 1..vote_count {
            threshold_candidate = threshold_candidate.wrapping_mul(MINSTD_RAND_MULTIPLIER);
            if threshold_candidate < min_threshold {
                min_threshold = threshold_candidate;
            }
        }
        threshold = min_threshold;
    }
    threshold
}

fn calculate_vdf_sortition_difficulty(
    params: crate::sortition::SortitionParams,
    threshold: u16,
) -> Result<u16> {
    const THRESHOLD_CORRECTION: u32 = 10;

    ensure!(
        params.vdf.difficulty_max >= params.vdf.difficulty_min,
        "VDF difficulty max must be greater than or equal to min"
    );
    let number_of_difficulties =
        u32::from(params.vdf.difficulty_max - params.vdf.difficulty_min) + 1;
    ensure!(
        number_of_difficulties != 0,
        "VDF difficulty range cannot be empty"
    );
    ensure!(
        u32::from(params.vrf.threshold_upper) >= number_of_difficulties,
        "VDF threshold upper must cover the difficulty range"
    );

    let corrected_threshold = u32::from(threshold) * THRESHOLD_CORRECTION;
    if corrected_threshold >= u32::from(params.vrf.threshold_upper) {
        Ok(params.vdf.difficulty_stale)
    } else {
        let bucket_width = u32::from(params.vrf.threshold_upper) / number_of_difficulties;
        ensure!(
            bucket_width != 0,
            "VDF difficulty bucket width cannot be zero"
        );
        Ok(params.vdf.difficulty_min + (corrected_threshold / bucket_width) as u16)
    }
}

/// Legacy VDF modulus bytes used by C++ `VdfSortition`.
///
/// C++ defines the modulus as `dev::asBytes("<hex text>")`, which preserves
/// the ASCII hex characters instead of decoding them into binary. Production
/// parity therefore requires using these exact 256 bytes until the VDF/VRF
/// module contract is deliberately migrated.
const LEGACY_VDF_MODULUS_ASCII_HEX: &[u8] =
    b"3d1055a514e17cce1290ccb5befb256b00b8aac664e39e754466fcd631004c9e23d16f23\
      9aee2a207e5173a7ee8f90ee9ab9b6a745d27c6e850e7ca7332388dfef7e5bbe6267d1f7\
      9f9330e44715b3f2066f903081836c1c83ca29126f8fdc5f5922bf3f9ddb4540171691ac\
      cc1ef6a34b2a804a18159c89c39b16edee2ede35";

fn exceeds_pbft_dag_count(tips_count: u64, dag_gas_limit: u64, pbft_gas_limit: u64) -> bool {
    let Some(max_dag_blocks) = pbft_gas_limit.checked_div(dag_gas_limit) else {
        return true;
    };
    tips_count.saturating_add(1) > max_dag_blocks
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagGraph {
    vertices: BTreeMap<H256, BTreeSet<H256>>,
}

impl DagGraph {
    pub fn new(genesis: H256) -> Self {
        assert_ne!(genesis, H256::zero(), "DAG genesis hash must not be zero");

        let mut graph = Self {
            vertices: BTreeMap::new(),
        };
        graph.add_vertex_edges(genesis, H256::zero(), &[]);
        graph
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    pub fn edge_count(&self) -> usize {
        self.vertices.values().map(BTreeSet::len).sum()
    }

    pub fn has_vertex(&self, vertex: H256) -> bool {
        self.vertices.contains_key(&vertex)
    }

    pub fn add_vertex_edges(&mut self, new_vertex: H256, pivot: H256, tips: &[H256]) -> bool {
        assert_ne!(new_vertex, H256::zero(), "DAG vertex hash must not be zero");

        self.vertices.entry(new_vertex).or_default();

        let mut inserted_all_edges = true;
        if pivot != H256::zero() && self.has_vertex(pivot) {
            inserted_all_edges &= self.add_edge(pivot, new_vertex);
        }

        for tip in tips {
            if self.has_vertex(*tip) {
                inserted_all_edges &= self.add_edge(*tip, new_vertex);
            }
        }

        inserted_all_edges
    }

    pub fn leaves(&self) -> Vec<H256> {
        self.vertices
            .iter()
            .filter_map(|(vertex, children)| children.is_empty().then_some(*vertex))
            .collect()
    }

    pub fn reachable(&self, from: H256, to: H256) -> bool {
        if from == to {
            return self.has_vertex(from);
        }
        if !self.has_vertex(from) || !self.has_vertex(to) {
            return false;
        }

        let mut stack = vec![from];
        let mut visited = BTreeSet::from([from]);

        while let Some(current) = stack.pop() {
            let Some(children) = self.vertices.get(&current) else {
                continue;
            };
            for child in children {
                if *child == to {
                    return true;
                }
                if visited.insert(*child) {
                    stack.push(*child);
                }
            }
        }

        false
    }

    pub fn ghost_path(&self, root: H256) -> Vec<H256> {
        if !self.has_vertex(root) {
            return Vec::new();
        }

        let weights = self.descendant_weights(root);
        let mut path = Vec::new();
        let mut current = root;

        loop {
            path.push(current);

            let Some(children) = self.vertices.get(&current) else {
                break;
            };
            let next = children
                .iter()
                .filter_map(|child| weights.get(child).map(|weight| (*child, *weight)))
                .max_by(|(left_hash, left_weight), (right_hash, right_weight)| {
                    left_weight
                        .cmp(right_weight)
                        .then_with(|| right_hash.cmp(left_hash))
                });

            let Some((next_hash, next_weight)) = next else {
                break;
            };
            if next_weight == 0 {
                break;
            }
            current = next_hash;
        }

        path
    }

    pub fn compute_order(
        &self,
        anchor: H256,
        non_finalized_blocks: &BTreeMap<u64, BTreeSet<H256>>,
    ) -> Option<Vec<H256>> {
        if !self.has_vertex(anchor) {
            return None;
        }

        let mut epoch_vertices = BTreeSet::from([anchor]);
        for block in non_finalized_blocks.values().flatten() {
            if self.reachable(*block, anchor) {
                epoch_vertices.insert(*block);
            }
        }

        let mut visited = BTreeSet::new();
        let mut ordered = Vec::new();

        for vertex in &epoch_vertices {
            if !visited.insert(*vertex) {
                continue;
            }

            let mut dfs = vec![(*vertex, false)];
            while let Some((current, post_order)) = dfs.pop() {
                if post_order {
                    ordered.push(current);
                    continue;
                }

                dfs.push((current, true));

                let mut neighbors: Vec<H256> = self
                    .vertices
                    .get(&current)
                    .into_iter()
                    .flatten()
                    .filter(|child| epoch_vertices.contains(child))
                    .filter(|child| visited.insert(**child))
                    .copied()
                    .collect();
                neighbors.sort();

                for neighbor in neighbors {
                    dfs.push((neighbor, false));
                }
            }
        }

        ordered.reverse();
        Some(ordered)
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
    }

    pub fn graphviz_dot(&self) -> String {
        let mut dot = String::from("digraph G {\n");
        for vertex in self.vertices.keys() {
            let _ = writeln!(
                dot,
                "  \"{}\" [label=\"{} \"];",
                hex_hash(vertex),
                hex_prefix(vertex)
            );
        }
        for (from, children) in &self.vertices {
            for child in children {
                let _ = writeln!(dot, "  \"{}\" -> \"{}\";", hex_hash(from), hex_hash(child));
            }
        }
        dot.push_str("}\n");
        dot
    }

    fn add_edge(&mut self, from: H256, to: H256) -> bool {
        match self.vertices.get_mut(&from) {
            Some(children) => children.insert(to),
            None => false,
        }
    }

    fn descendant_weights(&self, root: H256) -> BTreeMap<H256, usize> {
        let mut post_order = Vec::new();
        let mut stack = vec![root];

        while let Some(current) = stack.pop() {
            post_order.push(current);
            if let Some(children) = self.vertices.get(&current) {
                for child in children {
                    stack.push(*child);
                }
            }
        }
        post_order.reverse();

        let mut weights = BTreeMap::new();
        for vertex in post_order {
            let total_children_weight = self
                .vertices
                .get(&vertex)
                .into_iter()
                .flatten()
                .filter_map(|child| weights.get(child))
                .sum::<usize>();
            weights.insert(vertex, total_children_weight + 1);
        }

        weights
    }
}

/// Immutable block metadata used to update Rust-owned `DagManager` state.
///
/// Inputs:
/// - `hash`: DAG block hash. It must be nonzero.
/// - `pivot`: pivot parent hash, or zero for the current anchor root.
/// - `tips`: non-pivot parent hashes in block order.
/// - `level`: DAG level persisted on the block.
/// - `difficulty`: VDF difficulty used for non-finalized minimum-difficulty tracking.
///
/// Invariants:
/// - A block can be applied repeatedly without duplicating graph vertices or
///   non-finalized indexes.
/// - Missing parent hashes do not create edges, matching legacy graph behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagManagerBlock {
    pub hash: H256,
    pub pivot: H256,
    pub tips: Vec<H256>,
    pub level: u64,
    pub difficulty: u32,
}

/// Complete snapshot used to rebuild Rust-owned `DagManager` state from the
/// C++ side while DB, transaction, event, and network ownership still lives in
/// C++.
///
/// Inputs:
/// - anchors and period mirror the legacy manager state at one point in time.
/// - `anchor_level`, `max_level`, and `dag_expiry_level` preserve legacy
///   counters that are still affected by storage and finalization side effects.
/// - `non_finalized_min_difficulty` is accepted from C++ for exact parity during
///   transitional rebuilds; subsequent Rust `add_block` calls maintain it.
/// - `non_finalized_blocks` is the ordered set of currently live blocks that
///   should be present in the in-memory DAG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagManagerSnapshot {
    pub old_anchor: H256,
    pub anchor: H256,
    pub anchor_level: u64,
    pub period: u64,
    pub max_level: u64,
    pub dag_expiry_level: u64,
    pub non_finalized_min_difficulty: u32,
    pub non_finalized_blocks: Vec<DagManagerBlock>,
}

/// Deterministic effects produced by one finalized DAG order transition.
///
/// This type is the Rust domain contract for the stateful part of
/// `DagManager::setDagBlockOrder`. It contains only hash-level facts; storage,
/// transaction-manager, cache, event, and network side effects remain at the
/// runtime/shim boundary.
///
/// Fields:
/// - `previous_period` / `new_period`: period transition applied by the plan.
/// - `previous_anchor` / `current_anchor`: anchor transition applied by the
///   plan.
/// - `finalized_count`: legacy-compatible count of unique hashes supplied in
///   the finalized order.
/// - `dag_expiry_level`: expiry level after applying the anchor-level rule.
/// - `counter_update_hashes`: finalized-order hashes that were not present in
///   the in-memory non-finalized index before the transition and therefore need
///   persistent DAG counter/index updates for sync parity.
/// - `expired_hashes`: previously non-finalized blocks removed because they
///   are below the new expiry level or reference another expired block.
/// - `remaining_hashes`: non-finalized hashes that remain live after the
///   transition, sorted by level and hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagManagerFinalizationPlan {
    pub previous_period: u64,
    pub new_period: u64,
    pub previous_anchor: H256,
    pub current_anchor: H256,
    pub finalized_count: usize,
    pub dag_expiry_level: u64,
    pub counter_update_hashes: Vec<H256>,
    pub expired_hashes: Vec<H256>,
    pub remaining_hashes: Vec<H256>,
}

/// Rust-owned in-memory state for deterministic `DagManager` behavior.
///
/// This type owns the total DAG graph, pivot tree, non-finalized block index,
/// block levels, frontier, anchors, period, max level, expiry level, and
/// non-finalized minimum difficulty. It deliberately does not own storage,
/// transaction pool effects, verified-block events, or network gossip yet; the
/// C++ shim still performs those side effects and feeds successful state changes
/// into this object.
///
/// Output guarantees:
/// - Graph reads, frontier derivation, ghost path, block ordering, counters, and
///   pivot/tip metadata are derived from one Rust state object.
/// - Non-finalized block snapshots are returned in deterministic level/hash
///   order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagManagerState {
    total_dag: DagGraph,
    pivot_tree: DagGraph,
    block_levels: BTreeMap<H256, u64>,
    blocks: BTreeMap<H256, DagManagerBlock>,
    non_finalized_blocks: BTreeMap<u64, BTreeSet<H256>>,
    old_anchor: H256,
    anchor: H256,
    period: u64,
    max_level: u64,
    dag_expiry_limit: u32,
    dag_expiry_level: u64,
    non_finalized_min_difficulty: u32,
    frontier: DagFrontier,
}

impl DagManagerState {
    /// Creates a Rust-owned manager state rooted at the genesis DAG block.
    ///
    /// `genesis` must be nonzero. The initial state has period `0`, no
    /// non-finalized blocks, zero expiry level, and a frontier derived from the
    /// single genesis root.
    pub fn new(genesis: H256, dag_expiry_limit: u32) -> Result<Self> {
        if genesis == H256::zero() {
            bail!("DagManagerState requires a nonzero genesis hash");
        }

        let total_dag = DagGraph::new(genesis);
        let pivot_tree = DagGraph::new(genesis);
        let frontier = derive_frontier(&pivot_tree.ghost_path(genesis), &total_dag.leaves());
        let mut block_levels = BTreeMap::new();
        block_levels.insert(genesis, 0);

        Ok(Self {
            total_dag,
            pivot_tree,
            block_levels,
            blocks: BTreeMap::new(),
            non_finalized_blocks: BTreeMap::new(),
            old_anchor: H256::zero(),
            anchor: genesis,
            period: 0,
            max_level: 0,
            dag_expiry_limit,
            dag_expiry_level: 0,
            non_finalized_min_difficulty: u32::MAX,
            frontier,
        })
    }

    /// Replaces the current Rust state with a full snapshot from the C++ side.
    ///
    /// This is the transitional synchronization point after startup recovery and
    /// finalization, where storage cleanup and transaction side effects are
    /// still owned by C++.
    pub fn rebuild_from_snapshot(&mut self, snapshot: DagManagerSnapshot) -> Result<()> {
        if snapshot.anchor == H256::zero() {
            bail!("DagManagerState snapshot anchor must be nonzero");
        }

        self.total_dag.clear();
        self.pivot_tree.clear();
        self.block_levels.clear();
        self.blocks.clear();
        self.non_finalized_blocks.clear();

        self.old_anchor = snapshot.old_anchor;
        self.anchor = snapshot.anchor;
        self.period = snapshot.period;
        self.max_level = snapshot.max_level;
        self.dag_expiry_level = snapshot.dag_expiry_level;
        self.non_finalized_min_difficulty = snapshot.non_finalized_min_difficulty;

        self.block_levels
            .insert(snapshot.anchor, snapshot.anchor_level);
        self.total_dag
            .add_vertex_edges(snapshot.anchor, H256::zero(), &[]);
        self.pivot_tree
            .add_vertex_edges(snapshot.anchor, H256::zero(), &[]);

        for block in snapshot.non_finalized_blocks {
            self.add_non_finalized_block(block)?;
        }
        self.frontier = self.compute_frontier();

        Ok(())
    }

    /// Builds a fresh Rust DAG manager state from one snapshot.
    ///
    /// This is a convenience constructor for callers that create state from a
    /// persisted snapshot rather than mutating an existing instance.
    pub fn from_snapshot(snapshot: DagManagerSnapshot, dag_expiry_limit: u32) -> Result<Self> {
        let mut state = Self::new(snapshot.anchor, dag_expiry_limit)?;
        state.rebuild_from_snapshot(snapshot)?;
        Ok(state)
    }

    /// Adds one non-finalized block to the Rust-owned in-memory DAG state.
    ///
    /// The caller must invoke this only after C++ side validation, persistence,
    /// and transaction handling have succeeded. The method updates graph edges,
    /// block level metadata, non-finalized indexes, max level, min difficulty,
    /// and frontier.
    pub fn add_block(&mut self, block: DagManagerBlock) -> Result<()> {
        self.add_non_finalized_block(block)?;
        self.frontier = self.compute_frontier();
        Ok(())
    }

    /// Applies one finalized DAG order update and transitions to the next
    /// period/anchor.
    ///
    /// Inputs:
    /// - `new_anchor`: anchor hash for the new period (must be nonzero).
    /// - `new_period`: expected to be exactly `period + 1`.
    /// - `finalized_order`: hashes finalized by this period.
    /// - `new_anchor_level`: storage-resolved level for `new_anchor`.
    ///
    /// Output:
    /// - a deterministic finalization plan containing the legacy-compatible
    ///   finalized count and side-effect hashes for the runtime boundary.
    ///
    /// Behavior:
    /// - updates `old_anchor`, `anchor`, and `period`
    /// - removes finalized blocks from level indexes and block metadata
    /// - advances expiry level from the anchor level
    /// - removes non-finalized blocks that expired directly or through an
    ///   expired pivot/tip dependency
    /// - rebuilds DAG graphs and frontier from remaining non-finalized blocks
    pub fn set_finalized_order(
        &mut self,
        new_anchor: H256,
        new_period: u64,
        finalized_order: &[H256],
        new_anchor_level: u64,
    ) -> Result<DagManagerFinalizationPlan> {
        ensure!(new_anchor != H256::zero(), "new anchor must be nonzero");
        ensure!(
            new_period == self.period.saturating_add(1),
            "DAG_MANAGER_FINALIZATION_INVALID_PERIOD: expected {}, got {}",
            self.period.saturating_add(1),
            new_period
        );

        let previous_non_finalized = self
            .non_finalized_blocks
            .values()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();

        let previous_period = self.period;
        let previous_anchor = self.anchor;

        let mut finalized = BTreeSet::new();
        let mut counter_update_hashes = Vec::new();
        for hash in finalized_order {
            if finalized.insert(*hash) && !previous_non_finalized.contains(hash) {
                counter_update_hashes.push(*hash);
            }
        }
        ensure!(
            finalized.contains(&new_anchor),
            "DAG_MANAGER_FINALIZATION_ANCHOR_NOT_IN_ORDER: anchor {:?} missing from finalized order",
            new_anchor
        );

        for hash in &finalized {
            self.remove_non_finalized_block_metadata(*hash);
        }
        if new_anchor_level > u64::from(self.dag_expiry_limit) {
            self.dag_expiry_level = new_anchor_level - u64::from(self.dag_expiry_limit);
        }

        let remaining_hashes = self
            .non_finalized_blocks
            .values()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let mut remaining_set = BTreeSet::new();
        for hash in &remaining_hashes {
            remaining_set.insert(*hash);
        }

        let mut references = BTreeMap::new();
        for hash in &remaining_hashes {
            let Some(block) = self.blocks.get(hash) else {
                continue;
            };
            for ref_hash in std::iter::once(block.pivot).chain(block.tips.iter().copied()) {
                if remaining_set.contains(&ref_hash) {
                    references
                        .entry(ref_hash)
                        .or_insert_with(BTreeSet::new)
                        .insert(*hash);
                }
            }
        }

        let mut expired = BTreeSet::new();
        let mut expired_hashes = Vec::new();
        let mut queue = VecDeque::new();
        for hash in remaining_hashes.iter() {
            let Some(block) = self.blocks.get(hash) else {
                continue;
            };
            if block.level < self.dag_expiry_level {
                expired.insert(*hash);
                queue.push_back(*hash);
                expired_hashes.push(*hash);
            }
        }
        while let Some(expired_hash) = queue.pop_front() {
            if let Some(dependents) = references.get(&expired_hash) {
                for dependent in dependents {
                    if expired.insert(*dependent) {
                        queue.push_back(*dependent);
                        expired_hashes.push(*dependent);
                    }
                }
            }
        }
        for hash in &expired_hashes {
            self.remove_non_finalized_block_metadata(*hash);
        }

        self.old_anchor = self.anchor;
        self.anchor = new_anchor;
        self.period = new_period;
        self.block_levels.clear();
        self.block_levels.insert(self.anchor, new_anchor_level);
        for block in self.blocks.values() {
            self.block_levels.insert(block.hash, block.level);
        }
        self.rebuild_graphs_from_records()?;
        self.refresh_non_finalized_min_difficulty();
        self.frontier = self.compute_frontier();
        let remaining_hashes = self
            .non_finalized_blocks
            .values()
            .flatten()
            .copied()
            .collect::<Vec<_>>();

        Ok(DagManagerFinalizationPlan {
            previous_period,
            new_period,
            previous_anchor,
            current_anchor: new_anchor,
            finalized_count: finalized.len(),
            dag_expiry_level: self.dag_expiry_level,
            counter_update_hashes,
            expired_hashes,
            remaining_hashes,
        })
    }

    /// Advances the finalized period for an empty PBFT block.
    ///
    /// Inputs:
    /// - `new_period`: expected to be exactly `period + 1`.
    ///
    /// Behavior:
    /// - updates only the latest period, preserving current and previous
    ///   anchors because a null-anchor PBFT block does not finalize a new DAG
    ///   anchor in the legacy manager.
    ///
    /// Errors:
    /// - returns an error when the transition is not exactly sequential.
    pub fn advance_empty_period(&mut self, new_period: u64) -> Result<()> {
        ensure!(
            new_period == self.period.saturating_add(1),
            "invalid period transition: expected {}, got {}",
            self.period.saturating_add(1),
            new_period
        );
        self.period = new_period;
        Ok(())
    }

    /// Returns true when the total DAG mirror contains `hash`.
    pub fn has_vertex(&self, hash: H256) -> bool {
        self.total_dag.has_vertex(hash)
    }

    /// Returns reference metadata for pivot/tip validation from Rust state.
    pub fn reference_metadata(&self, hash: H256) -> DagReferenceMetadata {
        match self.block_levels.get(&hash).copied() {
            Some(level) if self.total_dag.has_vertex(hash) => DagReferenceMetadata {
                hash,
                found: true,
                level,
            },
            _ => DagReferenceMetadata {
                hash,
                found: false,
                level: 0,
            },
        }
    }

    /// Returns non-finalized block hashes in deterministic order, excluding hashes
    /// already known by the caller.
    ///
    /// Output order is ascending block level, then ascending hash within each
    /// level. `known_hashes` is treated as a set and may include duplicates
    /// without changing output.
    pub fn select_non_finalized_hashes_excluding_known(&self, known_hashes: &[H256]) -> Vec<H256> {
        let known = known_hashes.iter().copied().collect::<BTreeSet<_>>();
        self.non_finalized_blocks
            .values()
            .flat_map(|level_blocks| level_blocks.iter())
            .filter(|hash| !known.contains(hash))
            .copied()
            .collect()
    }

    /// Validates pivot/tip availability and level for a block using Rust state.
    pub fn validate_pivot_tips(
        &self,
        block_level: u64,
        pivot: H256,
        tips: &[H256],
    ) -> DagPivotTipsValidation {
        let pivot = self.reference_metadata(pivot);
        let tips = tips
            .iter()
            .map(|tip| self.reference_metadata(*tip))
            .collect::<Vec<_>>();
        validate_pivot_tips_metadata(block_level, pivot, &tips)
    }

    /// Computes DAG order for `anchor` from the Rust non-finalized index.
    pub fn compute_order(&self, anchor: H256) -> Option<Vec<H256>> {
        self.total_dag
            .compute_order(anchor, &self.non_finalized_blocks)
    }

    /// Returns the pivot ghost path from an explicit source.
    pub fn ghost_path(&self, source: H256) -> Vec<H256> {
        self.pivot_tree.ghost_path(source)
    }

    /// Returns the pivot ghost path from the current anchor.
    pub fn anchor_ghost_path(&self) -> Vec<H256> {
        self.pivot_tree.ghost_path(self.anchor)
    }

    /// Returns the cached frontier derived from current Rust graph state.
    pub fn frontier(&self) -> &DagFrontier {
        &self.frontier
    }

    /// Returns graph facts needed by the DAG proposer before live transaction selection.
    ///
    /// Output fields are derived from the Rust DAG mirror so the proposer does not perform C++ `DagBlock` lookups for
    /// frontier level, anchor, or non-finalized pressure checks. Missing frontier metadata intentionally maps to level
    /// `0`, matching the legacy proposer fallback and allowing callers to reject later through existing validation.
    pub fn proposer_frontier_facts(&self) -> DagProposerFrontierFacts {
        let mut max_frontier_level = self.reference_metadata(self.frontier.pivot).level;
        for tip in &self.frontier.tips {
            max_frontier_level = max_frontier_level.max(self.reference_metadata(*tip).level);
        }
        let (_, non_finalized_block_count) = self.non_finalized_blocks_size();

        DagProposerFrontierFacts {
            frontier: self.frontier.clone(),
            propose_level: max_frontier_level.saturating_add(1),
            anchor: self.anchor,
            non_finalized_block_count,
            non_finalized_min_difficulty: self.non_finalized_min_difficulty,
        }
    }

    /// Returns graphviz output for the total DAG when `pivot_tree == false`,
    /// otherwise for the pivot tree.
    pub fn graphviz_dot(&self, pivot_tree: bool) -> String {
        if pivot_tree {
            self.pivot_tree.graphviz_dot()
        } else {
            self.total_dag.graphviz_dot()
        }
    }

    /// Returns the persisted old/current anchors mirrored in Rust state.
    pub fn anchors(&self) -> (H256, H256) {
        (self.old_anchor, self.anchor)
    }

    /// Returns the current anchor hash.
    pub fn anchor(&self) -> H256 {
        self.anchor
    }

    /// Returns the previous anchor hash.
    pub fn old_anchor(&self) -> H256 {
        self.old_anchor
    }

    /// Returns the latest finalized PBFT period mirrored in Rust state.
    pub fn period(&self) -> u64 {
        self.period
    }

    /// Returns the max non-finalized DAG level mirrored in Rust state.
    pub fn max_level(&self) -> u64 {
        self.max_level
    }

    /// Returns the configured DAG expiry limit.
    pub fn dag_expiry_limit(&self) -> u32 {
        self.dag_expiry_limit
    }

    /// Returns the currently active DAG expiry level.
    pub fn dag_expiry_level(&self) -> u64 {
        self.dag_expiry_level
    }

    /// Alias accessor for current DAG expiry level.
    pub fn expiry_level(&self) -> u64 {
        self.dag_expiry_level
    }

    /// Returns the current non-finalized minimum difficulty.
    pub fn non_finalized_min_difficulty(&self) -> u32 {
        self.non_finalized_min_difficulty
    }

    /// Optional minimum difficulty for non-finalized blocks.
    pub fn min_difficulty(&self) -> Option<u32> {
        (self.non_finalized_min_difficulty != u32::MAX).then_some(self.non_finalized_min_difficulty)
    }

    /// Returns total graph vertex count.
    pub fn vertex_count(&self) -> usize {
        self.total_dag.vertex_count()
    }

    /// Returns total graph edge count.
    pub fn edge_count(&self) -> usize {
        self.total_dag.edge_count()
    }

    /// Returns non-finalized levels and hashes in deterministic order.
    pub fn non_finalized_blocks(&self) -> &BTreeMap<u64, BTreeSet<H256>> {
        &self.non_finalized_blocks
    }

    /// Per-block level lookup map for current anchor and non-finalized blocks.
    pub fn block_levels(&self) -> &BTreeMap<H256, u64> {
        &self.block_levels
    }

    /// Read-only access to total DAG mirror.
    pub fn total_dag(&self) -> &DagGraph {
        &self.total_dag
    }

    /// Read-only access to pivot-tree DAG mirror.
    pub fn pivot_tree(&self) -> &DagGraph {
        &self.pivot_tree
    }

    /// Returns `(number of levels, number of blocks)` for non-finalized state.
    pub fn non_finalized_blocks_size(&self) -> (usize, usize) {
        (
            self.non_finalized_blocks.len(),
            self.non_finalized_blocks.values().map(BTreeSet::len).sum(),
        )
    }

    fn add_non_finalized_block(&mut self, block: DagManagerBlock) -> Result<()> {
        if block.hash == H256::zero() {
            bail!("DagManagerState cannot add a zero DAG block hash");
        }

        if let Some(existing) = self.blocks.get(&block.hash) {
            ensure!(
                existing == &block,
                "DagManagerState cannot add conflicting metadata for hash {:?}",
                block.hash
            );
            return Ok(());
        }

        self.blocks.insert(block.hash, block.clone());

        self.block_levels.insert(block.hash, block.level);
        self.max_level = self.max_level.max(block.level);
        self.non_finalized_blocks
            .entry(block.level)
            .or_default()
            .insert(block.hash);
        self.non_finalized_min_difficulty = self.non_finalized_min_difficulty.min(block.difficulty);

        self.total_dag
            .add_vertex_edges(block.hash, block.pivot, &block.tips);
        self.pivot_tree
            .add_vertex_edges(block.hash, block.pivot, &[]);

        Ok(())
    }

    fn remove_non_finalized_block_metadata(&mut self, hash: H256) {
        self.blocks.remove(&hash);
        if let Some(level) = self.block_levels.remove(&hash) {
            let remove_level_entry = self
                .non_finalized_blocks
                .get_mut(&level)
                .map(|hashes| {
                    hashes.remove(&hash);
                    hashes.is_empty()
                })
                .unwrap_or(false);
            if remove_level_entry {
                self.non_finalized_blocks.remove(&level);
            }
        }
    }

    fn rebuild_graphs_from_records(&mut self) -> Result<()> {
        self.total_dag.clear();
        self.pivot_tree.clear();
        self.total_dag
            .add_vertex_edges(self.anchor, H256::zero(), &[]);
        self.pivot_tree
            .add_vertex_edges(self.anchor, H256::zero(), &[]);

        for hash in self.non_finalized_blocks.values().flatten() {
            let block = self.blocks.get(hash).with_context(|| {
                format!("missing non-finalized block metadata for hash {hash:?}")
            })?;
            self.total_dag
                .add_vertex_edges(block.hash, block.pivot, &block.tips);
            self.pivot_tree
                .add_vertex_edges(block.hash, block.pivot, &[]);
        }
        Ok(())
    }

    fn refresh_non_finalized_min_difficulty(&mut self) {
        self.non_finalized_min_difficulty = self
            .blocks
            .values()
            .map(|block| block.difficulty)
            .min()
            .unwrap_or(u32::MAX);
    }

    fn compute_frontier(&self) -> DagFrontier {
        derive_frontier(
            &self.pivot_tree.ghost_path(self.anchor),
            &self.total_dag.leaves(),
        )
    }
}

fn hex_hash(hash: &H256) -> String {
    hash.as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hex_prefix(hash: &H256) -> String {
    hash.as_bytes()
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_storage::{Config, StatusField, Storage};
    use rustaxa_vdf::prover::{CancellationToken, WesolowskiProver};
    use rustaxa_vdf::sortition::{self, LegacySortitionParams};
    use rustaxa_vdf::vrf::public_key_from_secret;

    fn h(value: u64) -> H256 {
        H256::from_low_u64_be(value)
    }

    fn set(values: impl IntoIterator<Item = H256>) -> BTreeSet<H256> {
        values.into_iter().collect()
    }

    fn dag_block_rlp_with_vdf(vdf_rlp: Vec<u8>, transactions: &[H256]) -> Vec<u8> {
        let mut stream = RlpStream::new_list(8);
        stream.append(&h(1));
        stream.append(&7_u64);
        stream.append(&123_u64);
        stream.append(&vdf_rlp);
        stream.begin_list(0);
        stream.begin_list(transactions.len());
        for transaction in transactions {
            stream.append(transaction);
        }
        stream.append(&vec![9_u8; 65]);
        stream.append(&42_u64);
        stream.out().to_vec()
    }

    fn dag_vdf_test_input(transactions: &[H256]) -> Vec<u8> {
        let mut stream = RlpStream::new();
        stream.append(&h(1));
        for transaction in transactions {
            stream.append(transaction);
        }
        stream.out().to_vec()
    }

    fn temp_storage(name: &str) -> Storage {
        let dir = std::env::temp_dir().join(format!(
            "{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Storage::new(Config::new(dir)).unwrap()
    }

    fn signed_dag_block_rlp(seed: u8, level: u64, gas_estimation: u64) -> Vec<u8> {
        let signing_key = SigningKey::from_slice(&[seed; 32]).expect("signing key");
        let mut block = DagBlock {
            pivot: h(1),
            level,
            timestamp: 123,
            vdf: vec![1, 2, 3],
            tips: vec![],
            transactions: vec![h(99)],
            signature: [0; 65],
            gas_estimation,
        };
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(block.signing_hash().as_bytes())
            .expect("sign dag block");
        block.signature[..64].copy_from_slice(&signature.to_bytes());
        block.signature[64] = recovery_id.to_byte();

        let mut stream = RlpStream::new_list(8);
        stream.append(&block.pivot);
        stream.append(&block.level);
        stream.append(&block.timestamp);
        stream.append(&block.vdf);
        stream.append_list(&block.tips);
        stream.append_list(&block.transactions);
        stream.append(&block.signature.as_ref());
        stream.append(&block.gas_estimation);
        stream.out().to_vec()
    }

    fn vdf_payload_rlp(difficulty: u16, proof: Vec<u8>, output: Vec<u8>) -> Vec<u8> {
        let mut stream = RlpStream::new_list(4);
        stream.append(&vec![0xAB_u8; 80]);
        stream.append(&proof);
        stream.append(&output);
        stream.append(&difficulty);
        stream.out().to_vec()
    }

    fn sortition_params_for_vdf_tests(difficulty: u16) -> crate::sortition::SortitionParams {
        crate::sortition::SortitionParams {
            vrf: crate::sortition::VrfParams {
                threshold_upper: u16::MAX,
            },
            vdf: crate::sortition::VdfParams {
                difficulty_min: difficulty,
                difficulty_max: difficulty,
                difficulty_stale: difficulty,
                lambda_bound: 128,
            },
        }
    }

    const SECRET_KEY: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn proposer_attempt_input() -> DagProposerAttemptInput {
        let vrf_key = public_key_from_secret(&SECRET_KEY).expect("public key from fixed secret");
        DagProposerAttemptInput {
            transaction_pool_size: 1,
            non_finalized_transaction_count: 0,
            max_non_finalized_transactions: 100,
            frontier: DagProposerFrontierFacts {
                frontier: DagFrontier {
                    pivot: h(1),
                    tips: vec![],
                },
                propose_level: 2,
                anchor: h(1),
                non_finalized_block_count: 0,
                non_finalized_min_difficulty: u32::MAX,
            },
            proposal_period_found: true,
            proposal_period: 3,
            last_finalized_period: 3,
            dag_expiry_level_limit: 100,
            period_block_hash_found: true,
            period_block_hash: h(9),
            wallet_vrf_public_key: vrf_key,
            wallet_vrf_secret: SECRET_KEY,
            authorization_facts: DagDposAuthorizationFacts {
                vrf_key: Some(vrf_key),
                vrf_key_found: true,
                sender_eligible_vote_count: 10,
                vdf_sortition_max_vote_count: 20,
                eligibility_status: DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
            },
            sortition_params: crate::sortition::SortitionParams {
                vrf: crate::sortition::VrfParams {
                    threshold_upper: u16::MAX,
                },
                vdf: crate::sortition::VdfParams {
                    difficulty_min: 3,
                    difficulty_max: 3,
                    difficulty_stale: 9,
                    lambda_bound: 128,
                },
            },
            max_non_finalized_dag_blocks: 100,
            max_non_finalized_dag_blocks_low_difficulty: 50,
            last_propose_level: 0,
            retry_count: 0,
            max_retry_count: 20,
            proposal_weight_limit: 1_000,
            total_transaction_shards: 4,
            node_transaction_shard: 2,
            shard_period_interval: 10,
        }
    }

    #[test]
    fn genesis_graph_has_one_vertex_and_no_edges() {
        let graph = DagGraph::new(h(1));

        assert_eq!(graph.vertex_count(), 1);
        assert_eq!(graph.edge_count(), 0);
        assert!(graph.has_vertex(h(1)));
        assert_eq!(graph.leaves(), vec![h(1)]);
    }

    #[test]
    fn repeated_vertex_insertion_does_not_duplicate_vertices_or_edges() {
        let mut graph = DagGraph::new(h(1));

        assert!(graph.add_vertex_edges(h(2), h(1), &[]));
        assert!(!graph.add_vertex_edges(h(2), h(1), &[]));

        assert_eq!(graph.vertex_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn missing_pivot_and_tips_do_not_create_edges() {
        let mut graph = DagGraph::new(h(1));

        assert!(graph.add_vertex_edges(h(2), h(99), &[h(98)]));

        assert_eq!(graph.vertex_count(), 2);
        assert_eq!(graph.edge_count(), 0);
        assert_eq!(graph.leaves(), vec![h(1), h(2)]);
    }

    #[test]
    fn leaf_collection_includes_isolated_vertices_and_is_hash_ordered() {
        let mut graph = DagGraph::new(h(10));

        graph.add_vertex_edges(h(3), H256::zero(), &[]);
        graph.add_vertex_edges(h(2), h(10), &[]);
        graph.add_vertex_edges(h(1), h(10), &[]);

        assert_eq!(graph.leaves(), vec![h(1), h(2), h(3)]);
    }

    #[test]
    fn reachability_handles_self_descendants_missing_and_disconnected_vertices() {
        let mut graph = DagGraph::new(h(1));
        graph.add_vertex_edges(h(2), h(1), &[]);
        graph.add_vertex_edges(h(3), h(2), &[]);
        graph.add_vertex_edges(h(4), H256::zero(), &[]);

        assert!(graph.reachable(h(1), h(1)));
        assert!(graph.reachable(h(1), h(3)));
        assert!(!graph.reachable(h(3), h(1)));
        assert!(!graph.reachable(h(4), h(3)));
        assert!(!graph.reachable(h(99), h(3)));
        assert!(!graph.reachable(h(3), h(99)));
    }

    #[test]
    fn ghost_path_prefers_heaviest_subtree_and_ties_by_smallest_hash() {
        let mut graph = DagGraph::new(h(1));
        graph.add_vertex_edges(h(3), h(1), &[]);
        graph.add_vertex_edges(h(2), h(1), &[]);
        graph.add_vertex_edges(h(4), h(3), &[]);
        graph.add_vertex_edges(h(5), h(3), &[]);

        assert_eq!(graph.ghost_path(h(1)), vec![h(1), h(3), h(4)]);
        assert_eq!(graph.ghost_path(h(99)), Vec::<H256>::new());
    }

    #[test]
    fn compute_order_returns_none_for_missing_anchor() {
        let graph = DagGraph::new(h(1));

        assert_eq!(graph.compute_order(h(99), &BTreeMap::new()), None);
    }

    #[test]
    fn compute_order_keeps_only_blocks_that_reach_anchor() {
        let mut graph = DagGraph::new(h(1));
        graph.add_vertex_edges(h(2), h(1), &[]);
        graph.add_vertex_edges(h(3), h(2), &[]);
        graph.add_vertex_edges(h(4), H256::zero(), &[]);

        let non_finalized = BTreeMap::from([(1, set([h(1), h(2), h(3), h(4)]))]);

        assert_eq!(
            graph.compute_order(h(3), &non_finalized),
            Some(vec![h(1), h(2), h(3)])
        );
    }

    #[test]
    fn compute_order_is_deterministic_for_conflux_fixture() {
        let mut graph = DagGraph::new(h(1));
        graph.add_vertex_edges(h(2), h(1), &[]);
        graph.add_vertex_edges(h(3), h(1), &[]);
        graph.add_vertex_edges(h(4), h(2), &[h(3)]);
        graph.add_vertex_edges(h(5), h(2), &[]);
        graph.add_vertex_edges(h(7), h(3), &[]);
        graph.add_vertex_edges(h(6), h(4), &[h(5), h(7)]);
        graph.add_vertex_edges(h(8), h(2), &[]);
        graph.add_vertex_edges(h(11), h(7), &[]);
        graph.add_vertex_edges(h(10), h(11), &[h(4)]);
        graph.add_vertex_edges(h(9), h(6), &[h(8), h(10)]);
        graph.add_vertex_edges(h(12), h(9), &[]);

        let non_finalized = BTreeMap::from([(1, set([h(8), h(9), h(10), h(11)]))]);

        assert_eq!(
            graph.compute_order(h(9), &non_finalized),
            Some(vec![h(11), h(10), h(8), h(9)])
        );
    }

    #[test]
    fn compute_order_is_stable_across_insertion_order() {
        let mut left = DagGraph::new(h(1));
        left.add_vertex_edges(h(2), h(1), &[]);
        left.add_vertex_edges(h(3), h(1), &[]);
        left.add_vertex_edges(h(4), h(2), &[h(3)]);

        let mut right = DagGraph::new(h(1));
        right.add_vertex_edges(h(3), h(1), &[]);
        right.add_vertex_edges(h(2), h(1), &[]);
        right.add_vertex_edges(h(4), h(2), &[h(3)]);

        let non_finalized = BTreeMap::from([(1, set([h(2), h(3), h(4)]))]);

        assert_eq!(
            left.compute_order(h(4), &non_finalized),
            right.compute_order(h(4), &non_finalized)
        );
    }

    #[test]
    fn clear_empties_graph_and_allows_rebuild() {
        let mut graph = DagGraph::new(h(1));
        graph.add_vertex_edges(h(2), h(1), &[]);

        graph.clear();
        assert_eq!(graph.vertex_count(), 0);
        assert_eq!(graph.edge_count(), 0);
        assert!(graph.leaves().is_empty());

        graph.add_vertex_edges(h(3), H256::zero(), &[]);
        assert_eq!(graph.vertex_count(), 1);
        assert_eq!(graph.leaves(), vec![h(3)]);
    }

    #[test]
    fn graphviz_dot_uses_current_graph_edges() {
        let mut graph = DagGraph::new(h(1));
        graph.add_vertex_edges(h(2), h(1), &[]);

        let dot = graph.graphviz_dot();

        assert!(
            dot.contains("\"0000000000000000000000000000000000000000000000000000000000000001\"")
        );
        assert!(dot.contains("\"0000000000000000000000000000000000000000000000000000000000000001\" -> \"0000000000000000000000000000000000000000000000000000000000000002\""));
    }

    #[test]
    fn frontier_derivation_returns_empty_when_ghost_path_is_empty() {
        let frontier = derive_frontier(&[], &[h(1), h(2)]);

        assert_eq!(frontier.pivot, H256::zero());
        assert_eq!(frontier.tips, Vec::<H256>::new());
    }

    #[test]
    fn frontier_derivation_removes_pivot_and_preserves_leaf_order() {
        let frontier = derive_frontier(&[h(10), h(20)], &[h(30), h(20), h(10), h(30), h(2)]);

        assert_eq!(frontier.pivot, h(20));
        assert_eq!(frontier.tips, vec![h(30), h(10), h(30), h(2)]);
    }

    #[test]
    fn pivot_tips_validation_reports_missing_references_and_expected_level() {
        let result = validate_pivot_tips_metadata(
            11,
            DagReferenceMetadata {
                hash: h(100),
                found: false,
                level: 0,
            },
            &[
                DagReferenceMetadata {
                    hash: h(101),
                    found: true,
                    level: 4,
                },
                DagReferenceMetadata {
                    hash: h(102),
                    found: false,
                    level: 0,
                },
                DagReferenceMetadata {
                    hash: h(103),
                    found: true,
                    level: 9,
                },
            ],
        );

        assert!(!result.ok);
        assert_eq!(result.expected_level, 10);
        assert!(!result.level_matches);
        assert_eq!(result.missing_references, vec![h(100), h(102)]);
    }

    #[test]
    fn pivot_tips_validation_succeeds_when_level_matches_and_no_missing() {
        let result = validate_pivot_tips_metadata(
            8,
            DagReferenceMetadata {
                hash: h(200),
                found: true,
                level: 5,
            },
            &[
                DagReferenceMetadata {
                    hash: h(201),
                    found: true,
                    level: 7,
                },
                DagReferenceMetadata {
                    hash: h(202),
                    found: true,
                    level: 6,
                },
            ],
        );

        assert!(result.ok);
        assert_eq!(result.expected_level, 8);
        assert!(result.level_matches);
        assert!(result.missing_references.is_empty());
    }

    #[test]
    fn pivot_tips_validation_wraps_level_like_cpp_unsigned_arithmetic() {
        let result = validate_pivot_tips_metadata(
            0,
            DagReferenceMetadata {
                hash: h(300),
                found: true,
                level: u64::MAX,
            },
            &[],
        );

        assert!(result.ok);
        assert_eq!(result.expected_level, 0);
        assert!(result.level_matches);
        assert!(result.missing_references.is_empty());
    }

    #[test]
    fn verify_precheck_rejects_tip_count_over_limit() {
        let result = validate_dag_verify_precheck(DagVerifyPrecheckInput {
            block_level: 10,
            pivot: h(1),
            tips: (2..=(DAG_BLOCK_MAX_TIPS as u64 + 2)).map(h).collect(),
            proposal_period_found: true,
            proposal_period: 7,
            dag_expiry_level: 0,
        });

        assert!(!result.continue_validation);
        assert_eq!(
            result.reject_code,
            DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION
        );
    }

    #[test]
    fn verify_precheck_rejects_duplicate_pivot_or_tip_reference() {
        let duplicate_pivot = validate_dag_verify_precheck(DagVerifyPrecheckInput {
            block_level: 10,
            pivot: h(1),
            tips: vec![h(2), h(1)],
            proposal_period_found: true,
            proposal_period: 7,
            dag_expiry_level: 0,
        });
        let duplicate_tip = validate_dag_verify_precheck(DagVerifyPrecheckInput {
            block_level: 10,
            pivot: h(1),
            tips: vec![h(2), h(2)],
            proposal_period_found: true,
            proposal_period: 7,
            dag_expiry_level: 0,
        });

        assert_eq!(
            duplicate_pivot.reject_code,
            DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION
        );
        assert_eq!(
            duplicate_tip.reject_code,
            DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION
        );
    }

    #[test]
    fn verify_precheck_rejects_missing_proposal_period_before_expiry() {
        let result = validate_dag_verify_precheck(DagVerifyPrecheckInput {
            block_level: 1,
            pivot: h(1),
            tips: vec![],
            proposal_period_found: false,
            proposal_period: 0,
            dag_expiry_level: 2,
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_AHEAD_BLOCK);
    }

    #[test]
    fn verify_precheck_rejects_expired_block() {
        let result = validate_dag_verify_precheck(DagVerifyPrecheckInput {
            block_level: 4,
            pivot: h(1),
            tips: vec![],
            proposal_period_found: true,
            proposal_period: 7,
            dag_expiry_level: 5,
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_EXPIRED_BLOCK);
    }

    #[test]
    fn verify_precheck_continues_for_remaining_validation() {
        let result = validate_dag_verify_precheck(DagVerifyPrecheckInput {
            block_level: 5,
            pivot: h(1),
            tips: vec![h(2), h(3)],
            proposal_period_found: true,
            proposal_period: 7,
            dag_expiry_level: 5,
        });

        assert!(result.continue_validation);
        assert_eq!(result.reject_code, 0);
        assert_eq!(result.proposal_period, 7);
    }

    #[test]
    fn verify_transaction_availability_rejects_missing_transactions() {
        let result =
            validate_dag_verify_transaction_availability(DagVerifyTransactionAvailabilityInput {
                expected_transactions: 3,
                resolved_transactions: 2,
            });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_MISSING_TRANSACTION);
    }

    #[test]
    fn verify_transaction_availability_continues_when_all_transactions_are_present() {
        let result =
            validate_dag_verify_transaction_availability(DagVerifyTransactionAvailabilityInput {
                expected_transactions: 3,
                resolved_transactions: 3,
            });

        assert!(result.continue_validation);
        assert_eq!(result.reject_code, 0);
    }

    #[test]
    fn verify_transaction_query_plan_preserves_missing_block_order() {
        let plan =
            plan_dag_verify_transaction_query(&[h(1), h(2), h(3), h(1), h(2)], &[h(2), h(9)]);

        assert_eq!(plan.query_hashes, vec![h(1), h(3)]);
    }

    #[test]
    fn non_finalized_transaction_query_plan_deduplicates_first_seen_order() {
        let plan =
            plan_non_finalized_transaction_query(&[vec![h(1), h(2), h(1)], vec![h(3), h(2)]]);

        assert_eq!(plan.query_hashes, vec![h(1), h(2), h(3)]);
    }

    #[test]
    fn expired_transaction_cleanup_skips_finalized_and_retained_refs() {
        let plan = plan_expired_transaction_cleanup(
            &[
                DagExpiredTransactionFact {
                    hash: h(1),
                    finalized: false,
                },
                DagExpiredTransactionFact {
                    hash: h(2),
                    finalized: true,
                },
                DagExpiredTransactionFact {
                    hash: h(3),
                    finalized: false,
                },
                DagExpiredTransactionFact {
                    hash: h(1),
                    finalized: false,
                },
            ],
            &[h(3)],
        );

        assert_eq!(plan.remove_hashes, vec![h(1)]);
    }

    #[test]
    fn dag_block_period_storage_lookup_reports_found_and_missing_rows() {
        let storage = temp_storage("rustaxa_consensus_dag_block_period_lookup");
        storage.dag().write_period(h(7), 12, 3).unwrap();

        let found = dag_block_period_from_storage(&storage, h(7)).unwrap();
        let missing = dag_block_period_from_storage(&storage, h(8)).unwrap();

        assert_eq!(
            found,
            DagBlockPeriodStorageLookup {
                found: true,
                period: 12,
                position: 3,
            }
        );
        assert_eq!(
            missing,
            DagBlockPeriodStorageLookup {
                found: false,
                period: 0,
                position: 0,
            }
        );
    }

    #[test]
    fn expired_transaction_cleanup_storage_collects_facts_and_retained_refs() {
        let storage = temp_storage("rustaxa_consensus_dag_expired_cleanup");
        storage
            .dag()
            .write(
                h(3),
                3,
                0,
                &dag_block_rlp_with_vdf(vec![0x11], &[h(1), h(2), h(1)]),
            )
            .unwrap();
        storage
            .dag()
            .write(h(4), 4, 0, &dag_block_rlp_with_vdf(vec![0x22], &[h(3)]))
            .unwrap();
        storage
            .dag()
            .write(h(6), 6, 0, &dag_block_rlp_with_vdf(vec![0x33], &[h(3)]))
            .unwrap();
        storage
            .transaction()
            .write_location(h(2), 7, 0, false)
            .unwrap();

        let payload =
            collect_expired_transaction_cleanup_from_storage(&storage, &[h(3), h(4)], &[h(6)])
                .unwrap();

        assert_eq!(
            payload.expired_transaction_facts,
            vec![
                DagExpiredTransactionFact {
                    hash: h(1),
                    finalized: false,
                },
                DagExpiredTransactionFact {
                    hash: h(2),
                    finalized: true,
                },
                DagExpiredTransactionFact {
                    hash: h(1),
                    finalized: false,
                },
                DagExpiredTransactionFact {
                    hash: h(3),
                    finalized: false,
                },
            ]
        );
        assert_eq!(payload.remove_hashes, vec![h(1)]);
    }

    #[test]
    fn non_finalized_sync_payload_storage_loads_blocks_and_dedupes_transactions() {
        let storage = temp_storage("rustaxa_consensus_dag_sync_payload");
        let block_a = dag_block_rlp_with_vdf(vec![0x11], &[h(1), h(2), h(1)]);
        let block_b = dag_block_rlp_with_vdf(vec![0x22], &[h(3), h(2)]);
        storage.dag().write(h(11), 11, 0, &block_a).unwrap();
        storage.dag().write(h(12), 12, 0, &block_b).unwrap();
        storage.transaction().write(h(1), &[0xa1]).unwrap();
        storage.transaction().write(h(2), &[0xa2]).unwrap();

        let payload =
            collect_non_finalized_sync_payload_from_storage(&storage, &[h(11), h(12)]).unwrap();

        assert_eq!(
            payload.blocks,
            vec![
                DagSyncBlockRlp {
                    hash: h(11),
                    block_rlp: block_a,
                },
                DagSyncBlockRlp {
                    hash: h(12),
                    block_rlp: block_b,
                },
            ]
        );
        assert_eq!(
            payload.transactions,
            vec![
                DagTransactionStorageLookup {
                    hash: h(1),
                    found: true,
                    finalized: false,
                    tx_rlp: vec![0xa1],
                },
                DagTransactionStorageLookup {
                    hash: h(2),
                    found: true,
                    finalized: false,
                    tx_rlp: vec![0xa2],
                },
                DagTransactionStorageLookup {
                    hash: h(3),
                    found: false,
                    finalized: false,
                    tx_rlp: Vec::new(),
                },
            ]
        );
    }

    #[test]
    fn finalization_cleanup_storage_applies_counter_and_expiry_batch() {
        let storage = temp_storage("rustaxa_consensus_dag_finalization_cleanup");
        storage
            .dag()
            .write(h(8), 8, 2, &dag_block_rlp_with_vdf(vec![0x88], &[]))
            .unwrap();
        storage
            .dag()
            .write(
                h(3),
                3,
                0,
                &dag_block_rlp_with_vdf(vec![0x11], &[h(1), h(2), h(1)]),
            )
            .unwrap();
        storage
            .dag()
            .write(h(4), 4, 0, &dag_block_rlp_with_vdf(vec![0x22], &[h(3)]))
            .unwrap();
        storage
            .dag()
            .write(h(6), 6, 0, &dag_block_rlp_with_vdf(vec![0x33], &[h(3)]))
            .unwrap();
        storage.transaction().write(h(1), &[0xa1]).unwrap();
        storage.transaction().write(h(2), &[0xa2]).unwrap();
        storage.transaction().write(h(3), &[0xa3]).unwrap();
        storage
            .transaction()
            .write_location(h(2), 7, 0, false)
            .unwrap();

        let payload =
            apply_finalization_cleanup_from_storage(&storage, &[h(8)], &[h(3), h(4)], &[h(6)])
                .unwrap();

        assert_eq!(
            payload.counter_updates,
            vec![DagFinalizedCounterUpdate {
                hash: h(8),
                level: 7,
                tips_count: 0,
            }]
        );
        assert_eq!(payload.expired_hashes, vec![h(3), h(4)]);
        assert_eq!(payload.remove_transaction_hashes, vec![h(1)]);
        assert!(storage.dag().by_hash_rlp_optional(h(3)).unwrap().is_none());
        assert!(storage.dag().by_hash_rlp_optional(h(4)).unwrap().is_none());
        assert!(storage.dag().by_hash_rlp_optional(h(6)).unwrap().is_some());
        assert!(storage.transaction().rlp(h(1)).unwrap().is_none());
        assert_eq!(storage.transaction().rlp(h(2)).unwrap(), Some(vec![0xa2]));
        assert_eq!(storage.transaction().rlp(h(3)).unwrap(), Some(vec![0xa3]));
        assert_eq!(
            storage
                .metadata()
                .status_field(StatusField::DagBlkCount as u8)
                .unwrap(),
            5
        );
        assert_eq!(
            storage
                .metadata()
                .status_field(StatusField::DagEdgeCount as u8)
                .unwrap(),
            7
        );
    }

    #[test]
    fn verify_vdf_prepare_rejects_when_vrf_key_is_missing() {
        let result = prepare_dag_verify_vdf(DagVerifyVdfPrepareInput {
            vrf_key_found: false,
            eligible_vote_count: 12,
            vdf_max_vote_count: 42,
        });

        assert!(!result.continue_validation);
        assert_eq!(
            result.reject_code,
            DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION
        );
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_MISSING_VRF_KEY);
    }

    #[test]
    fn verify_vdf_prepare_uses_supplied_max_vote_count() {
        let result = prepare_dag_verify_vdf(DagVerifyVdfPrepareInput {
            vrf_key_found: true,
            eligible_vote_count: 12,
            vdf_max_vote_count: 42,
        });

        assert!(result.continue_validation);
        assert_eq!(result.reject_code, 0);
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_CONTINUE);
        assert_eq!(result.vote_count, 12);
        assert_eq!(result.max_vote_count, 42);
    }

    #[test]
    fn verify_authorization_rejects_when_vdf_is_invalid() {
        let result = validate_dag_verify_authorization(DagVerifyAuthorizationInput {
            vdf_valid: false,
            dpos_snapshot_available: true,
            dpos_eligible: true,
        });

        assert!(!result.continue_validation);
        assert_eq!(
            result.reject_code,
            DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION
        );
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_INVALID_VDF);
    }

    #[test]
    fn verify_authorization_rejects_future_snapshot_before_not_eligible() {
        let result = validate_dag_verify_authorization(DagVerifyAuthorizationInput {
            vdf_valid: true,
            dpos_snapshot_available: false,
            dpos_eligible: false,
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_FUTURE_BLOCK);
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_FUTURE_DPOS_SNAPSHOT);
    }

    #[test]
    fn verify_authorization_rejects_not_eligible() {
        let result = validate_dag_verify_authorization(DagVerifyAuthorizationInput {
            vdf_valid: true,
            dpos_snapshot_available: true,
            dpos_eligible: false,
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_NOT_ELIGIBLE);
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_NOT_ELIGIBLE);
    }

    #[test]
    fn verify_authorization_continues_when_all_checks_pass() {
        let result = validate_dag_verify_authorization(DagVerifyAuthorizationInput {
            vdf_valid: true,
            dpos_snapshot_available: true,
            dpos_eligible: true,
        });

        assert!(result.continue_validation);
        assert_eq!(result.reject_code, 0);
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_CONTINUE);
    }

    #[test]
    fn verify_vdf_dpos_rejects_missing_vrf_before_other_facts() {
        let result = decide_dag_verify_vdf_dpos_authorization(DagVerifyVdfDposFacts {
            vrf_key_found: false,
            sender_eligible_vote_count: 12,
            vdf_sortition_max_vote_count: 42,
            vdf_status: DAG_VERIFY_VDF_STATUS_INVALID,
            dpos_status: DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE,
        });

        assert!(!result.continue_validation);
        assert_eq!(
            result.reject_code,
            DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION
        );
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_MISSING_VRF_KEY);
        assert_eq!(result.vote_count, 12);
        assert_eq!(result.max_vote_count, 42);
    }

    #[test]
    fn verify_vdf_dpos_rejects_invalid_vdf_before_dpos_facts() {
        let result = decide_dag_verify_vdf_dpos_authorization(DagVerifyVdfDposFacts {
            vrf_key_found: true,
            sender_eligible_vote_count: 12,
            vdf_sortition_max_vote_count: 42,
            vdf_status: DAG_VERIFY_VDF_STATUS_INVALID,
            dpos_status: DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE,
        });

        assert!(!result.continue_validation);
        assert_eq!(
            result.reject_code,
            DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION
        );
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_INVALID_VDF);
    }

    #[test]
    fn verify_vdf_dpos_rejects_future_snapshot_before_not_eligible() {
        let result = decide_dag_verify_vdf_dpos_authorization(DagVerifyVdfDposFacts {
            vrf_key_found: true,
            sender_eligible_vote_count: 12,
            vdf_sortition_max_vote_count: 42,
            vdf_status: DAG_VERIFY_VDF_STATUS_NOT_CHECKED,
            dpos_status: DAG_VERIFY_DPOS_STATUS_SNAPSHOT_UNAVAILABLE,
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_FUTURE_BLOCK);
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_FUTURE_DPOS_SNAPSHOT);
    }

    #[test]
    fn verify_vdf_dpos_continues_with_supplied_vote_counts() {
        let result = decide_dag_verify_vdf_dpos_authorization(DagVerifyVdfDposFacts {
            vrf_key_found: true,
            sender_eligible_vote_count: 12,
            vdf_sortition_max_vote_count: 42,
            vdf_status: DAG_VERIFY_VDF_STATUS_VALID,
            dpos_status: DAG_VERIFY_DPOS_STATUS_ELIGIBLE,
        });

        assert!(result.continue_validation);
        assert_eq!(result.reject_code, 0);
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_CONTINUE);
        assert_eq!(result.vote_count, 12);
        assert_eq!(result.max_vote_count, 42);
    }

    fn dag_block_with_vdf_payload(vdf_payload: Vec<u8>) -> Vec<u8> {
        let mut block = RlpStream::new_list(8);
        block.append(&h(1));
        block.append(&1u64);
        block.append(&0u64);
        block.append(&vdf_payload);
        block.begin_list(0);
        block.begin_list(0);
        block.append(&&[0u8; 65][..]);
        block.append(&123u64);
        block.out().to_vec()
    }

    #[test]
    fn dag_vrf_input_is_legacy_level_and_period_hash_rlp() {
        let block_level = 12_u64;
        let proposal_period_hash = h(99);
        let mut expected = RlpStream::new();
        expected.append(&block_level);
        expected.append(&proposal_period_hash);

        assert_eq!(
            construct_dag_vrf_input(block_level, proposal_period_hash),
            expected.out().to_vec()
        );
    }

    #[test]
    fn dag_block_transaction_hashes_preserves_order_and_duplicates() {
        let transactions = vec![h(1), h(2), h(1)];
        let block_rlp = dag_block_rlp_with_vdf(vec![0xAB_u8], &transactions);

        assert_eq!(
            dag_block_transaction_hashes(&block_rlp)
                .expect("transaction hashes should decode from DAG block payload"),
            transactions
        );
    }

    #[test]
    fn dag_vdf_message_is_pivot_and_transactions_rlp() {
        let transactions = vec![h(12), h(13)];
        let mut initial_payload = RlpStream::new_list(4);
        initial_payload.append(&vec![0x11_u8; 80]);
        initial_payload.append(&vec![0x22_u8]);
        initial_payload.append(&vec![0x33_u8]);
        initial_payload.append(&1u16);
        let block_rlp = dag_block_rlp_with_vdf(initial_payload.out().to_vec(), &transactions);

        let mut expected = RlpStream::new();
        expected.append(&h(1));
        for tx in &transactions {
            expected.append(tx);
        }

        assert_eq!(
            construct_dag_vdf_message_from_block_rlp(&block_rlp).unwrap(),
            expected.out().to_vec()
        );
    }

    #[test]
    fn dag_vdf_sortition_from_block_verifies_embedded_inputs() {
        let proposal_period_hash = h(77);
        let transactions = vec![h(12), h(13)];
        let block_level = 7_u64;
        let sortition_params = LegacySortitionParams {
            vrf_threshold_upper: 0x5ff,
            vdf_difficulty_min: 5,
            vdf_difficulty_max: 10,
            vdf_difficulty_stale: 9,
            vdf_lambda_bound: 64,
        };
        let sortition_params_for_input = crate::sortition::SortitionParams {
            vrf: crate::sortition::VrfParams {
                threshold_upper: 0x5ff,
            },
            vdf: crate::sortition::VdfParams {
                difficulty_min: 5,
                difficulty_max: 10,
                difficulty_stale: 9,
                lambda_bound: 64,
            },
        };

        let placeholder_payload =
            dag_block_rlp_with_vdf(vdf_payload_rlp(5, vec![1], vec![2]), &transactions);
        let vdf_input = construct_dag_vdf_message_from_block_rlp(&placeholder_payload).unwrap();
        let vrf_input = construct_dag_vrf_input(block_level, proposal_period_hash);
        let proof = sortition::prove_legacy_vdf_sortition(
            sortition_params,
            &SECRET_KEY,
            &vrf_input,
            &vdf_input,
            1,
            1,
            &CancellationToken::new(),
        )
        .expect("proof generation should succeed");

        let mut vdf_payload = RlpStream::new_list(4);
        vdf_payload.append(&&proof.vrf_proof[..]);
        vdf_payload.append(&proof.vdf_proof);
        vdf_payload.append(&proof.vdf_output);
        vdf_payload.append(&proof.difficulty);
        let block_rlp = dag_block_rlp_with_vdf(vdf_payload.out().to_vec(), &transactions);

        let result = verify_dag_vdf_sortition_from_block(DagVdfSortitionBlockInput {
            block_rlp,
            block_level,
            proposal_period_hash,
            vrf_public_key: public_key_from_secret(&SECRET_KEY)
                .expect("public key from fixed secret"),
            sortition_params: sortition_params_for_input,
            sender_eligible_vote_count: 1,
            vdf_sortition_max_vote_count: 1,
        })
        .unwrap();

        assert_eq!(result.vdf_status, DAG_VERIFY_VDF_STATUS_VALID);
        assert_eq!(result.difficulty, result.expected_difficulty);
    }

    #[test]
    fn dag_vdf_sortition_from_block_rejects_level_mismatch() {
        let transactions = vec![h(12), h(13)];
        let placeholder_payload =
            dag_block_rlp_with_vdf(vdf_payload_rlp(5, vec![1], vec![2]), &transactions);
        let err = verify_dag_vdf_sortition_from_block(DagVdfSortitionBlockInput {
            block_rlp: placeholder_payload,
            block_level: 999,
            proposal_period_hash: h(77),
            vrf_public_key: [0_u8; 32],
            sortition_params: crate::sortition::SortitionParams {
                vrf: crate::sortition::VrfParams {
                    threshold_upper: 1_000,
                },
                vdf: crate::sortition::VdfParams {
                    difficulty_min: 1,
                    difficulty_max: 1,
                    difficulty_stale: 1,
                    lambda_bound: 6,
                },
            },
            sender_eligible_vote_count: 100,
            vdf_sortition_max_vote_count: 100,
        })
        .expect_err("level mismatch should fail");

        assert!(err.to_string().contains("block level mismatch"));
    }

    #[test]
    fn verify_dag_vdf_sortition_rejects_invalid_payload() {
        let mut vdf_payload = RlpStream::new_list(3);
        vdf_payload.append(&vec![0x11u8; 80]);
        vdf_payload.append(&vec![0x22u8]);
        vdf_payload.append(&vec![0x33u8]);

        let result = verify_dag_vdf_sortition(DagVdfSortitionInput {
            block_rlp: dag_block_with_vdf_payload(vdf_payload.out().to_vec()),
            vdf_input: vec![0x01],
            sortition_params: crate::sortition::SortitionParams {
                vrf: crate::sortition::VrfParams {
                    threshold_upper: 1_000,
                },
                vdf: crate::sortition::VdfParams {
                    difficulty_min: 1,
                    difficulty_max: 1,
                    difficulty_stale: 1,
                    lambda_bound: 6,
                },
            },
            vrf_output: [0u8; 64],
            vrf_public_key: Vec::new(),
            vrf_input: Vec::new(),
            sender_eligible_vote_count: 100,
            vdf_sortition_max_vote_count: 100,
        })
        .expect_err("invalid VDF payload field count should be an operational error");

        assert!(
            result
                .to_string()
                .contains("decode DAG block VDF sortition payload")
        );
    }

    #[test]
    fn verify_dag_vdf_sortition_rejects_invalid_difficulty_or_proof_as_data() {
        let mut vdf_payload = RlpStream::new_list(4);
        vdf_payload.append(&vec![0x11u8; 80]);
        vdf_payload.append(&vec![0x22u8]);
        vdf_payload.append(&vec![0x33u8]);
        vdf_payload.append(&999u16);

        let result = verify_dag_vdf_sortition(DagVdfSortitionInput {
            block_rlp: dag_block_with_vdf_payload(vdf_payload.out().to_vec()),
            vdf_input: vec![0x01],
            sortition_params: crate::sortition::SortitionParams {
                vrf: crate::sortition::VrfParams {
                    threshold_upper: 1_000,
                },
                vdf: crate::sortition::VdfParams {
                    difficulty_min: 1,
                    difficulty_max: 1,
                    difficulty_stale: 1,
                    lambda_bound: 6,
                },
            },
            vrf_output: [0u8; 64],
            vrf_public_key: Vec::new(),
            vrf_input: Vec::new(),
            sender_eligible_vote_count: 100,
            vdf_sortition_max_vote_count: 100,
        })
        .expect("verification should complete");

        assert_eq!(result.vdf_status, DAG_VERIFY_VDF_STATUS_INVALID);
        assert_eq!(result.difficulty, 999);
        assert_eq!(result.expected_difficulty, 1);
    }

    #[test]
    fn dag_vdf_sortition_extracts_vrf_proof_from_block_rlp() {
        let block_rlp = dag_block_rlp_with_vdf(vdf_payload_rlp(4, vec![1], vec![2]), &[h(2)]);

        let proof = extract_dag_vdf_vrf_proof(&block_rlp).unwrap();

        assert_eq!(proof, [0xAB_u8; 80]);
    }

    #[test]
    fn dag_vdf_sortition_verifies_matching_difficulty_and_solution() {
        let transactions = vec![h(2), h(3)];
        let difficulty = 4_u16;
        let vdf = WesolowskiVdf::new(
            128,
            u32::from(difficulty),
            dag_vdf_test_input(&transactions),
            LEGACY_VDF_MODULUS_ASCII_HEX.to_vec(),
        );
        let solution = WesolowskiProver::new(&vdf).prove(&CancellationToken::new());
        let block_rlp = dag_block_rlp_with_vdf(
            vdf_payload_rlp(difficulty, solution.first, solution.second),
            &transactions,
        );

        let result = verify_dag_vdf_sortition(DagVdfSortitionInput {
            block_rlp,
            vdf_input: dag_vdf_test_input(&transactions),
            sortition_params: sortition_params_for_vdf_tests(difficulty),
            vrf_output: [0_u8; 64],
            vrf_public_key: Vec::new(),
            vrf_input: Vec::new(),
            sender_eligible_vote_count: 1,
            vdf_sortition_max_vote_count: 1000,
        })
        .unwrap();

        assert_eq!(result.vdf_status, DAG_VERIFY_VDF_STATUS_VALID);
        assert_eq!(result.difficulty, difficulty);
        assert_eq!(result.expected_difficulty, difficulty);
    }

    #[test]
    fn dag_vdf_sortition_verifies_embedded_vrf_proof() {
        let sortition_input = LegacySortitionParams {
            vrf_threshold_upper: 0x5ff,
            vdf_difficulty_min: 5,
            vdf_difficulty_max: 10,
            vdf_difficulty_stale: 9,
            vdf_lambda_bound: 64,
        };
        let vrf_input = vec![0xA1, 0x02, 0x03];
        let vdf_input = vec![0xB1, 0x04];
        let proof = sortition::prove_legacy_vdf_sortition(
            sortition_input,
            &SECRET_KEY,
            &vrf_input,
            &vdf_input,
            1,
            1,
            &CancellationToken::new(),
        )
        .expect("proof generation should succeed");

        let mut vdf_payload = RlpStream::new_list(4);
        vdf_payload.append(&&proof.vrf_proof[..]);
        vdf_payload.append(&proof.vdf_proof);
        vdf_payload.append(&proof.vdf_output);
        vdf_payload.append(&proof.difficulty);
        let block_rlp = dag_block_with_vdf_payload(vdf_payload.out().to_vec());

        let result = verify_dag_vdf_sortition(DagVdfSortitionInput {
            block_rlp,
            vdf_input,
            sortition_params: crate::sortition::SortitionParams {
                vrf: crate::sortition::VrfParams {
                    threshold_upper: 0x5ff,
                },
                vdf: crate::sortition::VdfParams {
                    difficulty_min: 5,
                    difficulty_max: 10,
                    difficulty_stale: 9,
                    lambda_bound: 64,
                },
            },
            vrf_output: [0_u8; 64],
            vrf_public_key: public_key_from_secret(&SECRET_KEY)
                .expect("public key from fixed secret")
                .to_vec(),
            vrf_input,
            sender_eligible_vote_count: 1,
            vdf_sortition_max_vote_count: 1,
        })
        .unwrap();

        assert_eq!(result.vdf_status, DAG_VERIFY_VDF_STATUS_VALID);
        assert_eq!(result.difficulty, result.expected_difficulty);
    }

    #[test]
    fn verify_gas_rejects_block_over_dag_limit() {
        let result = validate_dag_verify_gas(DagVerifyGasInput {
            block_gas_estimation: 101,
            estimated_transactions_weight: 101,
            dag_gas_limit: 100,
            pbft_gas_limit: 500,
            tip_gas_estimations: vec![],
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_BLOCK_TOO_BIG);
    }

    #[test]
    fn verify_gas_rejects_weight_mismatch() {
        let result = validate_dag_verify_gas(DagVerifyGasInput {
            block_gas_estimation: 90,
            estimated_transactions_weight: 91,
            dag_gas_limit: 100,
            pbft_gas_limit: 500,
            tip_gas_estimations: vec![],
        });

        assert!(!result.continue_validation);
        assert_eq!(
            result.reject_code,
            DAG_VERIFY_REJECT_INCORRECT_TRANSACTIONS_ESTIMATION
        );
    }

    #[test]
    fn verify_gas_rejects_missing_tip_when_pbft_aggregation_is_needed() {
        let result = validate_dag_verify_gas(DagVerifyGasInput {
            block_gas_estimation: 90,
            estimated_transactions_weight: 90,
            dag_gas_limit: 100,
            pbft_gas_limit: 200,
            tip_gas_estimations: vec![
                DagTipGas {
                    found: true,
                    gas_estimation: 70,
                },
                DagTipGas {
                    found: false,
                    gas_estimation: 0,
                },
            ],
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_MISSING_TIP);
    }

    #[test]
    fn verify_gas_rejects_tips_over_pbft_limit() {
        let result = validate_dag_verify_gas(DagVerifyGasInput {
            block_gas_estimation: 90,
            estimated_transactions_weight: 90,
            dag_gas_limit: 100,
            pbft_gas_limit: 200,
            tip_gas_estimations: vec![
                DagTipGas {
                    found: true,
                    gas_estimation: 70,
                },
                DagTipGas {
                    found: true,
                    gas_estimation: 50,
                },
            ],
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_BLOCK_TOO_BIG);
    }

    #[test]
    fn verify_gas_continues_when_all_checks_pass() {
        let result = validate_dag_verify_gas(DagVerifyGasInput {
            block_gas_estimation: 90,
            estimated_transactions_weight: 90,
            dag_gas_limit: 100,
            pbft_gas_limit: 300,
            tip_gas_estimations: vec![
                DagTipGas {
                    found: true,
                    gas_estimation: 80,
                },
                DagTipGas {
                    found: true,
                    gas_estimation: 70,
                },
            ],
        });

        assert!(result.continue_validation);
        assert_eq!(result.reject_code, 0);
    }

    #[test]
    fn dag_proposer_tip_selection_skips_missing_and_prefers_unique_higher_levels() {
        let candidates = vec![
            DagProposerTipCandidate {
                hash: h(1),
                found: true,
                sender: [0xA1; 20],
                level: 1,
                gas_estimation: 100,
            },
            DagProposerTipCandidate {
                hash: h(2),
                found: false,
                sender: [0; 20],
                level: 0,
                gas_estimation: 0,
            },
            DagProposerTipCandidate {
                hash: h(3),
                found: true,
                sender: [0xB1; 20],
                level: 2,
                gas_estimation: 100,
            },
            DagProposerTipCandidate {
                hash: h(4),
                found: true,
                sender: [0xB1; 20],
                level: 3,
                gas_estimation: 100,
            },
            DagProposerTipCandidate {
                hash: h(5),
                found: true,
                sender: [0xC1; 20],
                level: 1,
                gas_estimation: 100,
            },
        ];

        let selection = plan_dag_proposer_tip_selection(candidates, 250, 10);

        assert_eq!(selection.skipped_missing, 1);
        assert_eq!(selection.selected, vec![h(1), h(5)]);
    }

    #[test]
    fn dag_proposer_block_plan_keeps_unpruned_frontier_and_wraps_transaction_gas() {
        let plan = plan_dag_proposer_block_construction(DagProposerBlockConstructionInput {
            frontier_tips: vec![
                DagProposerTipCandidate {
                    hash: h(1),
                    found: false,
                    sender: [0; 20],
                    level: 0,
                    gas_estimation: 0,
                },
                DagProposerTipCandidate {
                    hash: h(2),
                    found: true,
                    sender: [0xA1; 20],
                    level: 4,
                    gas_estimation: 99,
                },
            ],
            transaction_gas_estimations: vec![u64::MAX, 3],
            pbft_gas_limit: 1_000,
            dag_gas_limit: 100,
            max_tips: 16,
        });

        assert_eq!(plan.selected_tips, vec![h(1), h(2)]);
        assert_eq!(plan.block_gas_estimation, 2);
        assert!(!plan.pruned_tips);
        assert_eq!(plan.skipped_missing_tips, 0);
    }

    #[test]
    fn dag_proposer_block_plan_prunes_with_remaining_pbft_gas() {
        let plan = plan_dag_proposer_block_construction(DagProposerBlockConstructionInput {
            frontier_tips: vec![
                DagProposerTipCandidate {
                    hash: h(1),
                    found: true,
                    sender: [0xA1; 20],
                    level: 1,
                    gas_estimation: 100,
                },
                DagProposerTipCandidate {
                    hash: h(2),
                    found: false,
                    sender: [0; 20],
                    level: 0,
                    gas_estimation: 0,
                },
                DagProposerTipCandidate {
                    hash: h(3),
                    found: true,
                    sender: [0xB1; 20],
                    level: 5,
                    gas_estimation: 400,
                },
                DagProposerTipCandidate {
                    hash: h(4),
                    found: true,
                    sender: [0xC1; 20],
                    level: 4,
                    gas_estimation: 200,
                },
            ],
            transaction_gas_estimations: vec![600],
            pbft_gas_limit: 1_000,
            dag_gas_limit: 250,
            max_tips: 16,
        });

        assert_eq!(plan.selected_tips, vec![h(3)]);
        assert_eq!(plan.block_gas_estimation, 600);
        assert!(plan.pruned_tips);
        assert_eq!(plan.skipped_missing_tips, 1);
    }

    #[test]
    fn dag_proposer_block_plan_loads_tip_metadata_from_storage() {
        let storage = temp_storage("rustaxa_consensus_dag_proposer_tip_metadata");
        let lower = h(10);
        let higher = h(20);

        storage
            .dag()
            .write(lower, 3, 0, &signed_dag_block_rlp(0x51, 3, 100))
            .expect("write lower tip");
        storage
            .dag()
            .write(higher, 5, 0, &signed_dag_block_rlp(0x52, 5, 100))
            .expect("write higher tip");

        let plan = plan_dag_proposer_block_construction_from_storage(
            &storage,
            DagProposerStorageBlockConstructionInput {
                frontier_tips: vec![lower, higher, h(30)],
                transaction_gas_estimations: vec![7],
                pbft_gas_limit: 1_000,
                dag_gas_limit: 1,
                max_tips: 1,
            },
        )
        .expect("plan");

        assert_eq!(plan.selected_tips, vec![higher]);
        assert_eq!(plan.block_gas_estimation, 7);
        assert!(plan.pruned_tips);
        assert_eq!(plan.skipped_missing_tips, 1);
    }

    #[test]
    fn dag_proposer_attempt_skips_before_fact_lookup_when_pool_empty() {
        let mut input = proposer_attempt_input();
        input.transaction_pool_size = 0;
        input.proposal_period_found = false;
        input.authorization_facts.vrf_key_found = false;

        let plan = plan_dag_proposer_attempt(input).expect("plan");

        assert_eq!(plan.action, DAG_PROPOSER_ACTION_SKIP);
        assert_eq!(plan.reason_code, DAG_PROPOSER_REASON_TRANSACTION_POOL_EMPTY);
        assert!(plan.vrf_input.is_empty());
    }

    #[test]
    fn dag_proposer_post_pack_resets_retry_state_for_empty_pack() {
        let plan = plan_dag_proposer_post_pack(DagProposerPostPackInput {
            proposal_level: 42,
            packed_transaction_count: 0,
        });

        assert_eq!(plan.action, DAG_PROPOSER_ACTION_SKIP);
        assert_eq!(
            plan.reason_code,
            DAG_PROPOSER_REASON_PACKED_TRANSACTIONS_EMPTY
        );
        assert!(plan.update_retry_state);
        assert_eq!(plan.next_last_propose_level, 42);
        assert_eq!(plan.next_retry_count, 0);
    }

    #[test]
    fn dag_proposer_post_pack_continues_for_non_empty_pack() {
        let plan = plan_dag_proposer_post_pack(DagProposerPostPackInput {
            proposal_level: 42,
            packed_transaction_count: 2,
        });

        assert_eq!(plan.action, DAG_PROPOSER_ACTION_CONTINUE);
        assert_eq!(plan.reason_code, DAG_PROPOSER_REASON_OK);
        assert!(!plan.update_retry_state);
    }

    #[test]
    fn dag_proposer_attempt_requests_transaction_pack_on_success() {
        let input = proposer_attempt_input();

        let plan = plan_dag_proposer_attempt(input).expect("plan");

        assert_eq!(plan.action, DAG_PROPOSER_ACTION_CONTINUE);
        assert_eq!(plan.reason_code, DAG_PROPOSER_REASON_OK);
        assert_eq!(plan.proposal_level, 2);
        assert_eq!(plan.proposal_period, 3);
        assert_eq!(plan.period_block_hash, h(9));
        assert!(!plan.vrf_input.is_empty());
        assert_eq!(plan.vote_count, 10);
        assert_eq!(plan.max_vote_count, 20);
        assert_eq!(plan.transaction_request.proposal_period, 3);
        assert_eq!(plan.transaction_request.weight_limit, 1_000);
        assert_eq!(plan.transaction_request.total_transaction_shards, 4);
        assert_eq!(plan.transaction_request.node_transaction_shard, 2);
        assert_eq!(plan.transaction_request.shard_period_interval, 10);
        assert!(!plan.update_retry_state);
    }

    #[test]
    fn dag_proposer_attempt_plans_stale_same_level_retry_update() {
        let mut input = proposer_attempt_input();
        input.sortition_params = sortition_params_for_vdf_tests(9);
        input.last_propose_level = 2;
        input.retry_count = 4;
        input.max_retry_count = 20;

        let plan = plan_dag_proposer_attempt(input).expect("plan");

        assert_eq!(plan.action, DAG_PROPOSER_ACTION_SKIP);
        assert_eq!(plan.reason_code, DAG_PROPOSER_REASON_STALE_VDF_RETRY);
        assert!(plan.vdf_stale);
        assert!(plan.update_retry_state);
        assert_eq!(plan.next_last_propose_level, 2);
        assert_eq!(plan.next_retry_count, 5);
    }

    fn record(hash: u64, pivot: u64, tips: &[u64], level: u64, difficulty: u64) -> DagManagerBlock {
        DagManagerBlock {
            hash: h(hash),
            pivot: h(pivot),
            tips: tips.iter().copied().map(h).collect(),
            level,
            difficulty: difficulty as u32,
        }
    }

    #[test]
    fn dag_manager_state_add_block_updates_indexes_and_frontier() {
        let mut state = DagManagerState::new(h(1), 0).expect("state");

        state.add_block(record(2, 1, &[], 2, 100)).expect("add");
        state.add_block(record(3, 2, &[1], 3, 50)).expect("add");

        assert_eq!(state.max_level(), 3);
        assert_eq!(state.min_difficulty(), Some(50));
        assert_eq!(state.frontier().pivot, h(3));
        assert!(state.frontier().tips.is_empty());
        assert_eq!(state.block_levels().get(&h(2)), Some(&2));
        assert_eq!(state.block_levels().get(&h(3)), Some(&3));
    }

    #[test]
    fn dag_manager_state_proposer_frontier_facts_use_rust_graph_metadata() {
        let mut state = DagManagerState::new(h(1), 0).expect("state");

        state.add_block(record(2, 1, &[], 2, 100)).expect("add");
        state.add_block(record(3, 2, &[1], 3, 80)).expect("add");
        state.add_block(record(4, 2, &[3], 4, 60)).expect("add");

        let facts = state.proposer_frontier_facts();

        assert_eq!(facts.frontier.pivot, h(3));
        assert_eq!(facts.frontier.tips, vec![h(4)]);
        assert_eq!(facts.propose_level, 5);
        assert_eq!(facts.anchor, h(1));
        assert_eq!(facts.non_finalized_block_count, 3);
        assert_eq!(facts.non_finalized_min_difficulty, 60);
    }

    #[test]
    fn dag_manager_state_rebuild_from_snapshot_restores_state() {
        let snapshot = DagManagerSnapshot {
            anchor: h(1),
            old_anchor: h(1),
            anchor_level: 0,
            period: 5,
            max_level: 9,
            dag_expiry_level: 4,
            non_finalized_min_difficulty: 60,
            non_finalized_blocks: vec![
                record(2, 1, &[], 2, 100),
                record(3, 2, &[1], 3, 80),
                record(4, 3, &[2], 4, 60),
            ],
        };

        let state = DagManagerState::from_snapshot(snapshot, 77).expect("snapshot");
        assert_eq!(state.anchor(), h(1));
        assert_eq!(state.old_anchor(), h(1));
        assert_eq!(state.period(), 5);
        assert_eq!(state.max_level(), 9);
        assert_eq!(state.expiry_level(), 4);
        assert_eq!(state.min_difficulty(), Some(60_u32));
        assert_eq!(state.frontier().pivot, h(4));
        assert!(state.frontier().tips.is_empty());
        assert!(state.total_dag().has_vertex(h(4)));
        assert!(state.pivot_tree().has_vertex(h(4)));
    }

    #[test]
    fn dag_manager_state_set_finalized_order_updates_anchor_and_rebuilds_graphs() {
        let mut state = DagManagerState::new(h(1), 0).expect("state");
        state.add_block(record(2, 1, &[], 2, 100)).expect("add");
        state.add_block(record(3, 2, &[], 3, 90)).expect("add");
        state.add_block(record(4, 2, &[3], 4, 80)).expect("add");

        let plan = state
            .set_finalized_order(h(4), 1, &[h(2), h(3), h(4)], 4)
            .expect("finalize");
        assert_eq!(plan.finalized_count, 3);
        assert_eq!(plan.previous_period, 0);
        assert_eq!(plan.new_period, 1);
        assert_eq!(plan.previous_anchor, h(1));
        assert_eq!(plan.current_anchor, h(4));
        assert_eq!(plan.dag_expiry_level, 4);
        assert!(plan.counter_update_hashes.is_empty());
        assert!(plan.expired_hashes.is_empty());
        assert!(plan.remaining_hashes.is_empty());
        assert_eq!(state.old_anchor(), h(1));
        assert_eq!(state.anchor(), h(4));
        assert_eq!(state.period(), 1);
        assert!(state.non_finalized_blocks().is_empty());
        assert_eq!(state.block_levels().len(), 1);
        assert_eq!(state.block_levels().get(&h(4)), Some(&4));
        assert_eq!(state.min_difficulty(), None);
        assert_eq!(state.frontier().pivot, h(4));
        assert!(state.frontier().tips.is_empty());
    }

    #[test]
    fn dag_manager_state_set_finalized_order_returns_legacy_unique_count() {
        let mut state = DagManagerState::new(h(1), 0).expect("state");
        state.add_block(record(2, 1, &[], 2, 100)).expect("add");
        state.add_block(record(3, 2, &[], 3, 90)).expect("add");

        let plan = state
            .set_finalized_order(h(3), 1, &[h(2), h(2), h(3), h(99)], 3)
            .expect("finalize");
        assert_eq!(plan.finalized_count, 3);
        assert_eq!(plan.counter_update_hashes, vec![h(99)]);
        assert_eq!(state.anchor(), h(3));
        assert!(state.non_finalized_blocks().is_empty());
    }

    #[test]
    fn dag_manager_state_set_finalized_order_prunes_expired_dependents() {
        let mut state = DagManagerState::new(h(1), 2).expect("state");
        state.add_block(record(2, 1, &[], 2, 100)).expect("add");
        state.add_block(record(3, 2, &[], 3, 90)).expect("add");
        state.add_block(record(4, 3, &[], 4, 80)).expect("add");
        state.add_block(record(5, 1, &[], 1, 70)).expect("add");
        state.add_block(record(6, 5, &[], 6, 60)).expect("add");

        let plan = state
            .set_finalized_order(h(4), 1, &[h(2), h(3), h(4)], 4)
            .expect("finalize");

        assert_eq!(state.expiry_level(), 2);
        assert_eq!(plan.dag_expiry_level, 2);
        assert_eq!(plan.expired_hashes, vec![h(5), h(6)]);
        assert!(plan.remaining_hashes.is_empty());
        assert!(state.non_finalized_blocks().is_empty());
    }

    #[test]
    fn dag_manager_state_set_finalized_order_requires_anchor_in_order() {
        let mut state = DagManagerState::new(h(1), 0).expect("state");
        state.add_block(record(2, 1, &[], 2, 100)).expect("add");

        let err = state
            .set_finalized_order(h(2), 1, &[], 2)
            .expect_err("anchor missing from order");
        assert!(format!("{err:#}").contains("DAG_MANAGER_FINALIZATION_ANCHOR_NOT_IN_ORDER"));
    }

    #[test]
    fn dag_manager_state_advance_empty_period_preserves_anchors() {
        let mut state = DagManagerState::new(h(1), 0).expect("state");
        state.add_block(record(2, 1, &[], 2, 100)).expect("add");

        state.advance_empty_period(1).expect("empty period");
        assert_eq!(state.old_anchor(), H256::zero());
        assert_eq!(state.anchor(), h(1));
        assert_eq!(state.period(), 1);
        assert!(state.non_finalized_blocks().contains_key(&2));
    }

    #[test]
    fn dag_manager_state_set_finalized_order_rejects_invalid_period_transition() {
        let mut state = DagManagerState::new(h(1), 0).expect("state");
        let snapshot = DagManagerSnapshot {
            anchor: h(1),
            old_anchor: h(1),
            anchor_level: 0,
            period: 2,
            max_level: 0,
            dag_expiry_level: 0,
            non_finalized_min_difficulty: u32::MAX,
            non_finalized_blocks: vec![],
        };
        state.rebuild_from_snapshot(snapshot).expect("snapshot");
        let err = state
            .set_finalized_order(h(2), 4, &[], 2)
            .expect_err("period transition must fail");
        assert!(format!("{err:#}").contains("DAG_MANAGER_FINALIZATION_INVALID_PERIOD"));
    }

    #[test]
    fn dag_manager_state_select_non_finalized_hashes_excludes_known() {
        let mut state = DagManagerState::new(h(1), 0).expect("state");
        state.add_block(record(4, 3, &[3], 4, 100)).expect("add");
        state.add_block(record(2, 1, &[], 2, 80)).expect("add");
        state.add_block(record(3, 2, &[1], 3, 90)).expect("add");
        state.add_block(record(6, 3, &[4], 4, 75)).expect("add");
        state.add_block(record(5, 3, &[4], 5, 70)).expect("add");

        let selected = state.select_non_finalized_hashes_excluding_known(&[h(3), h(2), h(3)]);
        assert_eq!(selected, vec![h(4), h(6), h(5)]);
    }
}
