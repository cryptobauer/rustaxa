use crate::concrete_state_projection::{
    FinalChainConcreteExecutionMarker, FinalChainConcreteState, FinalChainConcreteStateProvenance,
    concrete_state_bytes_digest, decode_concrete_execution_marker,
    decode_concrete_state_projection, decode_concrete_state_provenance,
    encode_concrete_execution_marker, encode_concrete_state_provenance,
};
use crate::final_chain::{
    DPOS_CONTRACT_ADDRESS, FinalChain, SLASHING_CONTRACT_ADDRESS,
    external_evm_pending_publication_marker,
};
use crate::rewards_stats::{
    FinalizedRewardsPeriodFact, RewardCertVoteFact, RewardDagBlockFact, RewardTransactionFact,
    RewardsStatsPeriodRlp,
};
use anyhow::{Context, ensure};
use ethereum_types::{H160, H256, U256};
use keccak_hasher::KeccakHasher;
use rustaxa_types::codec::rlp::final_chain::{
    LegacyBlockHeaderRlp, LegacyBlockHeaderRlpInput, StoredBlockHeaderRlp,
    StoredBlockHeaderRlpOwned,
};
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainGas, FinalChainGasPrice, FinalChainLogBloom, FinalChainNonce,
    FinalChainTransactionPosition, FinalChainTransactionValue, FinalizationDagBlock,
    FinalizationTransaction, LegacySystemTransactionInput, LegacyTransactionEnvelope,
    StoredFinalChainBlockHeader, encode_legacy_system_transaction,
};
use triehash::ordered_trie_root;

/// Explicit native-reference execution mode retained for focused Rust unit and
/// pure-reference coverage. Production finalization uses the concrete-enabled
/// mode so every period receives an exact concrete state root.
pub const FINAL_CHAIN_EXECUTION_MODE_NATIVE_ONLY: u8 = 0;
/// Production mode that transitions every finalized period through the
/// concrete EVM/state-db leaf, including empty, native-transfer, DPoS, and
/// slashing-only periods.
pub const FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED: u8 = 1;

/// Session is ready to expose the next execution step.
pub const FINAL_CHAIN_EXECUTION_STATUS_READY: u8 = 0;
/// Session completed a native Rust commit.
pub const FINAL_CHAIN_EXECUTION_STATUS_COMPLETE: u8 = 1;
/// Session is waiting for an external EVM execution report.
pub const FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM: u8 = 2;
/// Session rejected the request or report.
pub const FINAL_CHAIN_EXECUTION_STATUS_REJECTED: u8 = 3;
/// Session was aborted by its owner.
pub const FINAL_CHAIN_EXECUTION_STATUS_ABORTED: u8 = 4;
/// Session is waiting for post-execution external EVM rewards/state-root facts.
pub const FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_REWARDS: u8 = 5;
/// Session is waiting for bridge fact collection and Rust-planned system transaction RLPs.
pub const FINAL_CHAIN_EXECUTION_STATUS_WAITING_SYSTEM_TRANSACTIONS: u8 = 6;
/// Session has built external-EVM commit facts and is ready to derive a
/// publication plan, but live storage commit remains disabled.
pub const FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_PUBLICATION: u8 = 7;
/// Session has derived publication facts and is waiting for external EVM state
/// lifecycle confirmation.
pub const FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_LIFECYCLE: u8 = 8;
/// Session has accepted Rust-validated external EVM state commit intent and is
/// waiting for the executor boundary to report the actual staged-state outcome.
pub const FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_STATE_COMMIT: u8 = 9;
/// Session has accepted committed external EVM lifecycle facts and is waiting
/// for the Rust-owned FinalChain storage publication batch.
pub const FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_STORAGE_PUBLICATION: u8 = 10;

/// No work remains for the current session state.
pub const FINAL_CHAIN_EXECUTION_ACTION_COMPLETE: u8 = 0;
/// Commit the request through the Rust native/DPoS/slashing finalizer.
pub const FINAL_CHAIN_EXECUTION_ACTION_COMMIT_NATIVE: u8 = 1;
/// Execute arbitrary EVM transactions through an external executor port.
pub const FINAL_CHAIN_EXECUTION_ACTION_EXECUTE_EXTERNAL_EVM: u8 = 2;
/// Reject the request and keep FinalChain storage untouched.
pub const FINAL_CHAIN_EXECUTION_ACTION_REJECT: u8 = 3;
/// Distribute rewards through the external EVM executor boundary.
pub const FINAL_CHAIN_EXECUTION_ACTION_DISTRIBUTE_EXTERNAL_EVM_REWARDS: u8 = 4;
/// Provide system transactions before the external EVM executor request is
/// emitted.
pub const FINAL_CHAIN_EXECUTION_ACTION_PROVIDE_SYSTEM_TRANSACTIONS: u8 = 5;
/// Derive the Rust-owned external-EVM publication facts without mutating
/// storage.
pub const FINAL_CHAIN_EXECUTION_ACTION_PLAN_EXTERNAL_EVM_PUBLICATION: u8 = 6;
/// Report the external EVM staged-state lifecycle before storage publication.
pub const FINAL_CHAIN_EXECUTION_ACTION_REPORT_EXTERNAL_EVM_LIFECYCLE: u8 = 7;
/// Request Rust approval before committing externally staged EVM state.
pub const FINAL_CHAIN_EXECUTION_ACTION_REQUEST_EXTERNAL_EVM_STATE_COMMIT: u8 = 8;
/// Publish the Rust-owned external EVM FinalChain storage batch.
pub const FINAL_CHAIN_EXECUTION_ACTION_PUBLISH_EXTERNAL_EVM_STORAGE: u8 = 9;

/// Regular native value transfer.
pub const FINAL_CHAIN_EXECUTION_TX_KIND_NATIVE_VALUE_TRANSFER: u8 = 0;
/// Native Rust DPoS contract action.
pub const FINAL_CHAIN_EXECUTION_TX_KIND_DPOS_CONTRACT: u8 = 1;
/// Native Rust slashing contract action.
pub const FINAL_CHAIN_EXECUTION_TX_KIND_SLASHING_CONTRACT: u8 = 2;
/// External EVM contract call.
pub const FINAL_CHAIN_EXECUTION_TX_KIND_EXTERNAL_EVM_CALL: u8 = 3;
/// External EVM contract creation.
pub const FINAL_CHAIN_EXECUTION_TX_KIND_EXTERNAL_EVM_CREATE: u8 = 4;
/// Bridge-provided system transaction appended before external EVM execution.
pub const FINAL_CHAIN_EXECUTION_TX_KIND_SYSTEM: u8 = 5;

/// Successful external EVM report.
pub const FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS: u8 = 0;
/// External EVM executor rejected the requested execution.
pub const FINAL_CHAIN_EVM_REPORT_STATUS_REJECTED: u8 = 1;

/// Successful external EVM rewards/state-root report.
pub const FINAL_CHAIN_EVM_REWARDS_REPORT_STATUS_SUCCESS: u8 = 0;
/// External EVM rewards/state-root executor rejected the requested distribution.
pub const FINAL_CHAIN_EVM_REWARDS_REPORT_STATUS_REJECTED: u8 = 1;

/// External EVM staged state was committed by the executor boundary.
pub const FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED: u8 = 0;
/// External EVM staged state was discarded by the executor boundary.
pub const FINAL_CHAIN_EVM_LIFECYCLE_STATUS_DISCARDED: u8 = 1;
/// External EVM staged state was rejected or never reached a commit-capable state.
pub const FINAL_CHAIN_EVM_LIFECYCLE_STATUS_REJECTED: u8 = 2;

/// External EVM publication can be committed once storage publication is wired.
pub const FINAL_CHAIN_EVM_COMMIT_DECISION_READY_TO_PUBLISH: u8 = 0;
/// External EVM publication cannot be committed.
pub const FINAL_CHAIN_EVM_COMMIT_DECISION_REJECTED: u8 = 1;

/// External EVM staged state can be committed by the executor boundary.
pub const FINAL_CHAIN_EVM_STATE_COMMIT_INTENT_READY_TO_COMMIT: u8 = 0;
/// External EVM staged state must not be committed by the executor boundary.
pub const FINAL_CHAIN_EVM_STATE_COMMIT_INTENT_REJECTED: u8 = 1;

/// External EVM publication was applied to Rust FinalChain storage.
pub const FINAL_CHAIN_EVM_PUBLICATION_STATUS_APPLIED: u8 = 0;
/// External EVM publication was rejected before mutating storage.
pub const FINAL_CHAIN_EVM_PUBLICATION_STATUS_REJECTED: u8 = 1;
/// External EVM publication was already present with matching block indexes.
pub const FINAL_CHAIN_EVM_PUBLICATION_STATUS_ALREADY_APPLIED: u8 = 2;

/// A FinalChain snapshot was persisted or was already available for the publication period.
pub const FINAL_CHAIN_EVM_PUBLICATION_SNAPSHOT_STATUS_AVAILABLE: u8 = 0;
/// A FinalChain snapshot was not produced because the accepted external-EVM boundary cannot expose a full snapshot yet.
pub const FINAL_CHAIN_EVM_PUBLICATION_SNAPSHOT_STATUS_UNAVAILABLE_EXTERNAL_EVM_BOUNDARY: u8 = 1;
/// A FinalChain snapshot was not evaluated because publication was rejected or no block was published.
pub const FINAL_CHAIN_EVM_PUBLICATION_SNAPSHOT_STATUS_NOT_EVALUATED: u8 = 2;

/// External EVM publication audit matched all persisted FinalChain rows.
pub const FINAL_CHAIN_EVM_PUBLICATION_AUDIT_STATUS_MATCHED: u8 = 0;
/// External EVM publication audit found a missing or mismatched persisted row.
pub const FINAL_CHAIN_EVM_PUBLICATION_AUDIT_STATUS_MISMATCH: u8 = 1;

/// Recovery may publish the durable marker because concrete state committed
/// the exact planned period and post-rewards root.
pub const FINAL_CHAIN_EVM_RECOVERY_DECISION_READY_TO_PUBLISH: u8 = 0;
/// Recovery found the exact FinalChain block already durable and may clear the
/// duplicate marker without publishing it again.
pub const FINAL_CHAIN_EVM_RECOVERY_DECISION_ALREADY_PUBLISHED: u8 = 1;
/// Recovery proved concrete state never advanced beyond the exact prior
/// descriptor, so the abandoned marker may be cleared without publication.
pub const FINAL_CHAIN_EVM_RECOVERY_DECISION_CLEAR_UNCOMMITTED: u8 = 2;
/// Recovery facts are missing, stale, conflicting, ahead, or otherwise
/// ambiguous; the durable marker must remain for operator-visible recovery.
pub const FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED: u8 = 3;

/// Complete FinalChain execution request owned by a runtime session.
///
/// The payload preserves the existing bridge facts: signed PBFT block RLP,
/// finalized transactions, finalized DAG blocks, reward-rate context, and
/// certificate-vote facts. The runtime classifies the transaction set before
/// committing so native Rust execution and arbitrary EVM execution remain
/// separate boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalChainExecutionRequest {
    pub pbft_block_rlp: Vec<u8>,
    pub transactions: Vec<FinalizationTransaction>,
    pub finalized_dag_blocks: Vec<FinalizationDagBlock>,
    pub blocks_per_year: u32,
    pub cert_votes: Vec<RewardCertVoteFact>,
    pub block_gas_limit: FinalChainGas,
    pub mode: u8,
}

/// One transaction in the ordered block execution stream.
///
/// When any arbitrary EVM transaction is present, the runtime exposes every
/// bridge-provided finalized transaction in block order, including native value
/// transfers and Rust-native contract actions. The executor must return
/// matching positions and hashes for the full ordered request; mismatches are
/// treated as report forgery or stale work. System transactions are generated
/// by Rust from bridge-contract state facts and appended before the external
/// EVM request is emitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalChainEvmTransactionInput {
    pub position: FinalChainTransactionPosition,
    pub hash: [u8; 32],
    pub sender: [u8; 20],
    pub receiver: Option<[u8; 20]>,
    pub nonce: FinalChainNonce,
    pub value: FinalChainTransactionValue,
    pub gas_price: FinalChainGasPrice,
    pub gas_limit: FinalChainGas,
    pub data: Vec<u8>,
    pub rlp: Vec<u8>,
    pub kind: u8,
    pub is_system: bool,
}

/// Request for system transactions needed before external EVM execution.
///
/// Rust emits this request when arbitrary EVM work is present. The temporary
/// bridge owner answers by collecting bridge-contract state facts, asking Rust
/// to plan canonical system transaction RLPs, and reporting those exact RLPs
/// back through the session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainSystemTransactionRequest {
    pub request_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub regular_transaction_count: u64,
}

/// System transaction RLPs reported by the bridge boundary.
///
/// The report is side-effect-free. Rust validates identity and period, decodes
/// every payload using the fixed Taraxa system sender, and rejects malformed
/// bytes before constructing the external EVM request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainSystemTransactionReport {
    pub request_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub transactions: Vec<Vec<u8>>,
}

/// Bridge-contract state facts needed to plan external-EVM system transactions.
///
/// C++ still owns `StateAPI` reads and the `shouldFinalizeEpoch()` dry run.
/// Rust owns the deterministic gate and C++-compatible unsigned
/// `finalizeEpoch()` system transaction RLP construction from those facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainSystemTransactionPlanFact {
    pub request_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub is_pillar_block_period: bool,
    pub bridge_contract_address: [u8; 20],
    pub bridge_contract_found: bool,
    pub bridge_contract_has_code: bool,
    pub should_finalize_epoch: bool,
    pub system_account_nonce: FinalChainNonce,
    pub block_gas_limit: FinalChainGas,
}

/// Rust-planned system transaction RLPs for an external-EVM session.
///
/// The returned bytes are ready to report through
/// [`final_chain_execution_session_report_system_transactions`] and to
/// materialize temporarily as C++ `SystemTransaction` objects for `StateAPI`
/// execution. Rust does not read bridge state or execute EVM here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainSystemTransactionPlan {
    pub request_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub transactions: Vec<Vec<u8>>,
}

/// External EVM execution request emitted by a FinalChain runtime session.
///
/// `request_id` is deterministic over the exact concrete prior descriptor and
/// ordered transaction stream and must be echoed by the executor report. The
/// request exposes no state-trie handle; the concrete leaf receives only the
/// period, prior root, block facts, and canonical transactions it must execute.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainEvmExecutionRequest {
    pub request_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    /// Exact concrete descriptor from which this transition must start.
    ///
    /// It is bound into `request_id`; a plan prepared against another root is
    /// a different operation even when the PBFT block and transactions match.
    pub prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    /// Exact StateAPI marker that must be durably staged before `BeginBlock`.
    pub concrete_marker_rlp: Vec<u8>,
    /// Rust-owned plan identity echoed by every later concrete leaf.
    pub concrete_plan_hash: [u8; 32],
    /// Digest of the exact canonical ordered transaction stream.
    pub transactions_hash: [u8; 32],
    /// Digest of the deterministic rewards inputs known before execution.
    pub rewards_hash: [u8; 32],
    pub block_author: [u8; 20],
    pub timestamp: u64,
    pub block_gas_limit: FinalChainGas,
    pub transactions: Vec<FinalChainEvmTransactionInput>,
}

/// One log topic emitted by an external EVM transaction result.
///
/// The wrapper keeps CXX bridge payloads plain while preserving the exact
/// 32-byte topic shape used by the legacy receipt/log bloom path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalChainEvmLogTopic {
    pub topic: [u8; 32],
}

/// One structured log emitted by an external EVM transaction result.
///
/// Structured logs travel with receipt RLP so Rust can validate and eventually
/// publish receipt/log-bloom state without reparsing legacy C++ objects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalChainEvmLog {
    pub address: [u8; 20],
    pub topics: Vec<FinalChainEvmLogTopic>,
    pub data: Vec<u8>,
}

/// One transaction result reported by an external EVM executor.
///
/// The runtime validates identity, ordering, cumulative gas, typed receipt
/// agreement, and exact concrete transition roots. Reports do not alter
/// storage; only a later committed lifecycle descriptor can authorize the
/// Rust-owned publication path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalChainEvmTransactionResult {
    pub position: FinalChainTransactionPosition,
    pub hash: [u8; 32],
    pub status: u8,
    pub gas_used: FinalChainGas,
    pub cumulative_gas_used: FinalChainGas,
    pub receipt_rlp: Vec<u8>,
    pub logs: Vec<FinalChainEvmLog>,
    pub new_contract_address: Option<[u8; 20]>,
    /// Exact EVM return bytes (`ExecutionResult::CodeRetval`) reported by the
    /// concrete executor. These bytes are not part of the receipt, so they must
    /// remain explicit to bind the host report to StateAPI's durable projection.
    pub output: Vec<u8>,
    pub code_error: String,
    pub consensus_error: String,
}

/// External EVM execution report returned to a runtime session.
///
/// Reports are validated against the exact request emitted by the session.
/// Successful reports are still rejected for commit until EVM state-root and
/// receipt parity are wired and covered by differential tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainEvmExecutionReport {
    pub request_id: [u8; 32],
    pub status: u8,
    /// Concrete descriptor actually used by the executor. It must match the
    /// request exactly; reporting only a successful status is insufficient.
    pub prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    pub concrete_marker_rlp: Vec<u8>,
    pub concrete_plan_hash: [u8; 32],
    pub transactions_hash: [u8; 32],
    pub rewards_hash: [u8; 32],
    /// Exact concrete root after the ordered transaction stream and before
    /// rewards. Synthetic or unavailable roots are rejected.
    pub post_transaction_state_root: [u8; 32],
    pub cumulative_gas_used: FinalChainGas,
    pub results: Vec<FinalChainEvmTransactionResult>,
}

/// Rewards request emitted after a valid external EVM execution report.
///
/// The request keeps rewards outside FinalChain execution while giving the
/// future C++/Rust executor adapter enough deterministic facts to run the same
/// post-transaction rewards distribution boundary as legacy C++.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainEvmRewardsRequest {
    pub request_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    pub post_transaction_state_root: [u8; 32],
    pub concrete_marker_rlp: Vec<u8>,
    pub concrete_plan_hash: [u8; 32],
    pub transactions_hash: [u8; 32],
    pub rewards_hash: [u8; 32],
    pub block_author: [u8; 20],
    pub block_gas_used: FinalChainGas,
    pub transaction_gas_used: Vec<FinalChainGas>,
    pub transaction_fees: Vec<Vec<u8>>,
    pub finalized_dag_block_count: u64,
    /// Legacy-compatible per-period `BlockStats` RLP selected by the
    /// FinalChain-owned rewards runtime for this distribution boundary.
    pub distribution_stats: Vec<RewardsStatsPeriodRlp>,
}

/// Rewards-stat plan prepared by FinalChain for one exact external-EVM session.
///
/// The expected head and runtime generation prevent a plan from being reused
/// after another finalization advances either durable storage or the live
/// rewards cache. The storage mutation remains session-owned until publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FinalChainPreparedExternalEvmRewardsStatsPlan {
    pub(crate) request_id: [u8; 32],
    pub(crate) period: FinalChainBlockNumber,
    pub(crate) expected_prior_head: FinalChainBlockNumber,
    pub(crate) expected_runtime_generation: u64,
    pub(crate) distribution_stats: Vec<RewardsStatsPeriodRlp>,
    pub(crate) storage_update: FinalChainExternalEvmRewardsStatsUpdate,
}

/// Rewards/state-root facts returned by the external EVM executor boundary.
///
/// `state_root` is the post-rewards root that will eventually enter the
/// FinalChain block header. `total_reward` is the legacy total-reward header
/// field encoded as an unsigned big-endian integer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalChainEvmRewardsReport {
    pub request_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub status: u8,
    pub prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    pub post_transaction_state_root: [u8; 32],
    pub post_rewards_state_root: [u8; 32],
    pub concrete_marker_rlp: Vec<u8>,
    pub concrete_plan_hash: [u8; 32],
    pub transactions_hash: [u8; 32],
    pub rewards_hash: [u8; 32],
    /// Canonical 13-field StateAPI projection after rewards preparation.
    pub concrete_projection_rlp: Vec<u8>,
    pub concrete_projection_hash: [u8; 32],
    pub concrete_provenance_rlp: Vec<u8>,
    pub total_reward: Vec<u8>,
}

/// Non-mutating Rust plan for one concrete-EVM FinalChain commit.
///
/// Rust derives header and storage-publication facts from typed EVM and rewards
/// reports without touching `StateAPI`, `state_db/`, or FinalChain storage. The
/// exact prior, post-transaction, and post-rewards roots remain attached until
/// the concrete committed descriptor is validated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmCommitPlan {
    pub request_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    pub post_transaction_state_root: [u8; 32],
    pub post_rewards_state_root: [u8; 32],
    pub concrete_marker_rlp: Vec<u8>,
    pub concrete_plan_hash: [u8; 32],
    pub transactions_hash: [u8; 32],
    pub rewards_hash: [u8; 32],
    pub concrete_projection_rlp: Vec<u8>,
    pub concrete_projection_hash: [u8; 32],
    pub concrete_provenance_rlp: Vec<u8>,
    pub total_reward: Vec<u8>,
    pub transactions_root: [u8; 32],
    pub receipts_root: [u8; 32],
    pub header_log_bloom: FinalChainLogBloom,
    pub indexed_log_bloom: FinalChainLogBloom,
    pub receipts_rlp: Vec<u8>,
    pub encoded_receipts: Vec<Vec<u8>>,
    pub gas_used: FinalChainGas,
    pub executed_dag_blocks: u64,
    pub executed_transactions: u64,
    pub regular_transaction_count: u64,
    pub system_transaction_count: u64,
    pub error_code: String,
}

/// Transaction publication fact derived from an external-EVM commit plan.
///
/// Each item maps one transaction hash to its finalized block position,
/// system-transaction marker, canonical transaction RLP, and canonical receipt
/// RLP. The transaction payload lets native publication persist system
/// transactions atomically with their location and receipt indexes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmTransactionPublication {
    pub transaction_hash: [u8; 32],
    pub position: FinalChainTransactionPosition,
    pub is_system: bool,
    pub transaction_rlp: Vec<u8>,
    pub receipt_rlp: Vec<u8>,
}

/// Rewards-stat cache mutation to publish with an external-EVM block.
///
/// The external executor still distributes rewards through the C++ `StateAPI`,
/// but rewards-stat planning already runs through the Rust rewards runtime.
/// These fields carry only the storage cache mutation that must be committed
/// atomically with FinalChain block visibility.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmRewardsStatsUpdate {
    pub current_period: FinalChainBlockNumber,
    pub cache_current_period: bool,
    pub clear_cached_stats: bool,
    pub current_block_stats_rlp: Vec<u8>,
}

/// Optional proposal-period DAG-level boundary to publish with an external-EVM block.
///
/// C++ still owns temporary DAG anchor object materialization, but the derived
/// `(anchor level + max_levels_per_period) -> finalized period` storage row
/// belongs with the Rust FinalChain publication batch. `has_update` preserves
/// compatibility for publication plans and pending markers that do not carry an
/// anchor-derived mapping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinalChainProposalPeriodDagLevelUpdate {
    pub has_update: bool,
    pub level: u64,
}

/// Non-mutating publication plan for an external-EVM FinalChain block.
///
/// The plan materializes the stored-header RLP, legacy full header RLP, block
/// hash, receipt payloads, transaction index facts, system-transaction hash
/// list, optional proposal-period DAG-level mapping, and rewards-stat cache
/// mutation that Rust publishes in one storage batch after a safe external EVM
/// state lifecycle report. `plan_id` is a
/// deterministic hash of those publication facts and must be echoed by the
/// lifecycle report so stale staged-state decisions cannot be replayed across
/// blocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmPublicationPlan {
    pub request_id: [u8; 32],
    pub plan_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub block_hash: [u8; 32],
    pub block_header_rlp: Vec<u8>,
    pub stored_header_rlp: Vec<u8>,
    pub receipts_rlp: Vec<u8>,
    pub indexed_log_bloom: FinalChainLogBloom,
    pub system_transaction_hashes_rlp: Vec<u8>,
    pub transaction_publications: Vec<FinalChainExternalEvmTransactionPublication>,
    pub executed_dag_blocks: u64,
    pub executed_transactions: u64,
    pub proposal_period_dag_level_update: FinalChainProposalPeriodDagLevelUpdate,
    pub rewards_stats_update: FinalChainExternalEvmRewardsStatsUpdate,
    /// Rust-derived consensus projection validated against the native prefix
    /// of the concrete StateAPI receipt transcript.
    pub dpos_snapshot_rlp: Vec<u8>,
    /// Full synchronized native account snapshot derived from exact concrete
    /// per-transaction/final deltas. This must publish atomically with the block.
    pub account_snapshot_rlp: Vec<u8>,
    pub concrete_marker_rlp: Vec<u8>,
    pub concrete_projection_rlp: Vec<u8>,
    pub concrete_projection_hash: [u8; 32],
    pub concrete_provenance_rlp: Vec<u8>,
    pub error_code: String,
}

/// Rust-validated request to commit externally staged EVM state.
///
/// C++ builds this after EVM execution, reward distribution, and Rust
/// publication planning, but before calling `StateAPI::transition_state_commit`.
/// Rust validates only immutable identity and root facts; it does not commit,
/// discard, or inspect staged EVM state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmStateCommitRequest {
    pub request_id: [u8; 32],
    pub plan_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    pub post_transaction_state_root: [u8; 32],
    pub post_rewards_state_root: [u8; 32],
    pub publication_block_hash: [u8; 32],
    pub concrete_marker_rlp: Vec<u8>,
    pub concrete_projection_rlp: Vec<u8>,
    pub concrete_projection_hash: [u8; 32],
    pub concrete_provenance_rlp: Vec<u8>,
}

/// Rust decision that an external EVM state commit is safe to attempt.
///
/// A ready intent is not permission to publish FinalChain storage. The executor
/// boundary must first report the actual staged-state lifecycle outcome with
/// the same facts, and only a committed report can produce a publish decision.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmStateCommitIntent {
    pub request_id: [u8; 32],
    pub plan_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub publication_block_hash: [u8; 32],
    pub prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    pub post_transaction_state_root: [u8; 32],
    /// Exact root that the concrete state-db commit must make durable.
    pub post_rewards_state_root: [u8; 32],
    /// Compatibility alias for the marker codec while its durable version is
    /// upgraded to encode the complete lifecycle facts.
    pub expected_state_root: [u8; 32],
    pub concrete_marker_rlp: Vec<u8>,
    pub concrete_projection_rlp: Vec<u8>,
    pub concrete_projection_hash: [u8; 32],
    pub concrete_provenance_rlp: Vec<u8>,
    pub status: u8,
    pub error_code: String,
}

/// External EVM state-commit result reported by the executor boundary.
///
/// This is the narrow production boundary after Rust has already accepted a
/// [`FinalChainExternalEvmStateCommitIntent`]. The executor echoes the exact
/// identity and lifecycle roots plus the descriptor it observes after the
/// commit/discard attempt. Rust rejects missing or mismatched facts; a status
/// alone never authorizes publication or marker deletion.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmStateCommitResult {
    pub request_id: [u8; 32],
    pub plan_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub publication_block_hash: [u8; 32],
    pub prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    pub post_transaction_state_root: [u8; 32],
    pub post_rewards_state_root: [u8; 32],
    pub concrete_marker_rlp: Vec<u8>,
    pub concrete_projection_rlp: Vec<u8>,
    pub concrete_projection_hash: [u8; 32],
    /// Provenance atomically committed by StateAPI with the descriptor.
    pub concrete_provenance_rlp: Vec<u8>,
    /// Descriptor observed after the commit/discard call. Committed outcomes
    /// require the planned period/root; discarded outcomes require the exact
    /// prior descriptor. `None` is always ambiguous and never publish-safe.
    pub committed_state: Option<FinalChainExternalEvmCommittedStateDescriptor>,
    pub status: u8,
    pub error_code: String,
}

/// External EVM staged-state lifecycle report.
///
/// The external executor owns `StateAPI`, `state_db/`, staged commit/discard,
/// and any reward-state mutation. Rust validates only stable identity and
/// descriptor facts: request id, period, prior root, post-transaction root,
/// post-rewards root, observed committed descriptor, publication block hash,
/// lifecycle status, and error code. It never receives EVM handles or state
/// diffs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmLifecycleReport {
    pub request_id: [u8; 32],
    pub plan_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    pub post_transaction_state_root: [u8; 32],
    pub post_rewards_state_root: [u8; 32],
    pub publication_block_hash: [u8; 32],
    pub committed_state: Option<FinalChainExternalEvmCommittedStateDescriptor>,
    pub status: u8,
    pub error_code: String,
}

/// Final Rust decision after external EVM lifecycle validation.
///
/// A ready decision proves Rust consensus has validated every deterministic
/// fact needed for external-EVM FinalChain publication after the executor
/// reports committed staged state. `decision_id` is derived by Rust from the
/// accepted post-commit lifecycle facts; the publication API rejects zero or
/// mismatched ids so a state-commit intent cannot be accidentally reused as a
/// publish decision. The function returning this type still does not write
/// storage; storage publication remains an explicit follow-up boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmCommitDecision {
    pub request_id: [u8; 32],
    pub plan_id: [u8; 32],
    pub decision_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub publication_block_hash: [u8; 32],
    pub status: u8,
    pub error_code: String,
}

/// Result of applying an external-EVM publication plan to FinalChain storage.
///
/// The report is returned only by the explicit publication API. It does not
/// execute EVM or interact with `StateAPI`; it records whether the Rust-owned
/// FinalChain batch applied, was already present, or was rejected before any
/// storage mutation. Applied/already-applied reports carry the absolute
/// execution counters persisted by Rust storage after the publication. Snapshot
/// status fields intentionally distinguish persisted snapshots from facts that
/// remain unavailable at the accepted external-EVM boundary; callers must not
/// treat unavailable account snapshots as current account state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmPublicationReport {
    pub request_id: [u8; 32],
    pub plan_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub block_hash: [u8; 32],
    pub executed_dag_block_count: u64,
    pub executed_transaction_count: u64,
    pub dpos_snapshot_status: u8,
    pub account_snapshot_status: u8,
    pub status: u8,
    pub error_code: String,
    /// Startup recovery requires the executor to discard exactly the staged
    /// StateAPI marker described by the following fields before retrying the
    /// same Rust recovery operation. False for every other report.
    pub recovery_discard_required: bool,
    pub recovery_request_id: [u8; 32],
    pub recovery_period: FinalChainBlockNumber,
    pub recovery_concrete_marker_rlp: Vec<u8>,
    pub recovery_marker_hash: [u8; 32],
    pub recovery_prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    pub recovery_concrete_chain_identity: [u8; 32],
}

/// External state descriptor observed at the StateAPI/state-db boundary.
///
/// The descriptor is supplied by the external executor adapter after state DB
/// commit or restart. Rust uses it only as a compact audit/recovery fact: the
/// committed period must match the publication period and the committed root
/// must match the post-rewards root accepted by the Rust execution session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmCommittedStateDescriptor {
    pub period: FinalChainBlockNumber,
    pub state_root: [u8; 32],
}

/// Durable facts used to arbitrate one pending concrete-state publication.
///
/// This is deliberately a read-only planning input. The caller loads the
/// marker, FinalChain rows, and concrete committed descriptor; Rust validates
/// their exact identity and returns a decision without clearing a marker or
/// publishing storage.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmRecoveryFact {
    pub lifecycle_id: [u8; 32],
    pub request_id: [u8; 32],
    pub plan_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub publication_block_hash: [u8; 32],
    pub prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    pub post_transaction_state_root: [u8; 32],
    pub post_rewards_state_root: [u8; 32],
    pub finalized_head: FinalChainExternalEvmCommittedStateDescriptor,
    pub finalized_block_hash: Option<[u8; 32]>,
    pub finalized_block_state: Option<FinalChainExternalEvmCommittedStateDescriptor>,
    pub committed_state: Option<FinalChainExternalEvmCommittedStateDescriptor>,
    /// Exact StateAPI marker authored by Rust and embedded in the durable
    /// pending-publication plan before concrete commit.
    pub expected_concrete_marker_rlp: Vec<u8>,
    /// Exact provenance bytes authored by Rust for the planned committed
    /// descriptor. A committed or already-published recovery is valid only
    /// when StateAPI returns these identical bytes.
    pub expected_concrete_provenance_rlp: Vec<u8>,
    /// Provenance bytes observed from the reopened StateAPI database.
    pub observed_concrete_provenance_rlp: Vec<u8>,
    /// Pending StateAPI execution marker observed on reopen. Committed recovery
    /// requires this to be empty; an uncommitted staged transition must echo
    /// `expected_concrete_marker_rlp` exactly before it can be discarded.
    pub pending_concrete_marker_rlp: Vec<u8>,
}

/// Non-mutating recovery decision for one durable lifecycle marker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmRecoveryDecision {
    pub lifecycle_id: [u8; 32],
    pub request_id: [u8; 32],
    pub plan_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub publication_block_hash: [u8; 32],
    pub status: u8,
    pub error_code: String,
}

/// Exact read-only preflight for a concrete external-EVM execution.
///
/// Rust supplies the expected prior FinalChain descriptor. The state-db leaf
/// returns what is actually committed without advancing pending state. Native
/// execution fails closed unless periods match and, after genesis, roots match.
/// Genesis roots differ between the native and concrete state encodings, so
/// the first external block validates the genesis period but not root bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmPreflightRequest {
    pub request_id: [u8; 32],
    pub next_period: FinalChainBlockNumber,
    pub expected_prior: FinalChainExternalEvmCommittedStateDescriptor,
    /// Rust-owned stable chain identity used to activate/verify StateAPI policy.
    pub concrete_chain_identity: [u8; 32],
}

/// Observed concrete state-db descriptor for one preflight request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmPreflightReport {
    pub request_id: [u8; 32],
    pub committed: FinalChainExternalEvmCommittedStateDescriptor,
    /// Current durable StateAPI provenance. Empty is invalid once concrete-root
    /// policy is active.
    pub concrete_provenance_rlp: Vec<u8>,
    /// Exact pending StateAPI marker on reopen, or empty when no staged work exists.
    pub pending_concrete_marker_rlp: Vec<u8>,
    pub succeeded: bool,
    pub error_code: String,
}

/// Exact request to discard one StateAPI staged transition and reopen at its prior root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmDiscardRequest {
    pub request_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub concrete_marker_rlp: Vec<u8>,
    pub marker_hash: [u8; 32],
    pub prior_state: FinalChainExternalEvmCommittedStateDescriptor,
}

/// Echoed discard result. Success requires the exact prior committed descriptor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmDiscardReport {
    pub request_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub concrete_marker_rlp: Vec<u8>,
    pub marker_hash: [u8; 32],
    pub prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    pub committed_state: FinalChainExternalEvmCommittedStateDescriptor,
    pub succeeded: bool,
    pub error_code: String,
}

/// Result of auditing an external-EVM publication plan against storage.
///
/// The audit is read-only and is intended for parity and smoke coverage after
/// live publication or restart recovery. It checks that the Rust-owned
/// publication batch materialized the exact header, hash indexes, receipts,
/// transaction indexes, bloom index leaf, system-transaction hash row,
/// pending-marker state, and optional external committed-state descriptor
/// described by the plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmPublicationAuditReport {
    pub request_id: [u8; 32],
    pub plan_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub block_hash: [u8; 32],
    pub checked_fields: u64,
    pub status: u8,
    pub error_code: String,
}

/// Next action for a FinalChain execution session.
///
/// `status` describes the session state after producing the step. `action`
/// tells the caller what boundary to drive next. `evm_request` is populated only
/// when `action == FINAL_CHAIN_EXECUTION_ACTION_EXECUTE_EXTERNAL_EVM`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExecutionStep {
    pub status: u8,
    pub action: u8,
    pub period: FinalChainBlockNumber,
    pub external_evm_transaction_count: u64,
    pub evm_request: FinalChainEvmExecutionRequest,
    pub evm_rewards_request: FinalChainEvmRewardsRequest,
    pub system_transaction_request: FinalChainSystemTransactionRequest,
    pub error_code: String,
}

/// Result of committing a FinalChain execution session.
///
/// Native commits return the canonical block header RLP and receipt RLPs needed
/// by the C++ shim to keep its public `FinalizationResult` API stable. Rejected
/// commits leave storage unchanged and carry an explicit error code.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExecutionCommitReport {
    pub status: u8,
    pub period: FinalChainBlockNumber,
    pub block_header_rlp: Vec<u8>,
    pub receipts: Vec<Vec<u8>>,
    pub gas_used: FinalChainGas,
    pub executed_dag_blocks: u64,
    pub executed_transactions: u64,
    pub error_code: String,
}

/// Rust-owned runtime session for one FinalChain finalization request.
///
/// The session classifies the transaction set once. Explicit reference-mode
/// requests may retain the native commit path, while production
/// external-enabled requests always expose a concrete transition, including
/// empty or native-supported periods. The session validates every descriptor
/// and report before publication may proceed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalChainExecutionSession {
    request: FinalChainExecutionRequest,
    metadata: rustaxa_types::PbftBlockMetadata,
    block_number: FinalChainBlockNumber,
    evm_request: Option<FinalChainEvmExecutionRequest>,
    status: u8,
    system_transaction_request: Option<FinalChainSystemTransactionRequest>,
    system_transactions: Vec<FinalChainEvmTransactionInput>,
    report: Option<FinalChainEvmExecutionReport>,
    rewards_request: Option<FinalChainEvmRewardsRequest>,
    pub(crate) prepared_rewards_stats_plan: Option<FinalChainPreparedExternalEvmRewardsStatsPlan>,
    external_evm_commit_plan: Option<FinalChainExternalEvmCommitPlan>,
    external_evm_publication_plan: Option<FinalChainExternalEvmPublicationPlan>,
    external_evm_state_commit_intent: Option<FinalChainExternalEvmStateCommitIntent>,
    external_evm_commit_decision: Option<FinalChainExternalEvmCommitDecision>,
    error_code: String,
}

/// Creates a FinalChain execution session after decoding PBFT metadata and
/// classifying the transaction set.
///
/// Invalid PBFT RLP or unsupported native-only EVM transactions are represented
/// as rejected sessions instead of panicking. Native-supported transactions are
/// ready for an immediate Rust commit step.
pub fn create_final_chain_execution_session(
    request: FinalChainExecutionRequest,
) -> FinalChainExecutionSession {
    match rustaxa_types::PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(
        &request.pbft_block_rlp,
    ))
    .context("decode signed PBFT block metadata")
    {
        Ok(metadata) => FinalChainExecutionSession::new(request, metadata),
        Err(error) => FinalChainExecutionSession::rejected(
            request,
            rustaxa_types::PbftBlockMetadata {
                author: Default::default(),
                period: 0u64,
                timestamp: 0,
                extra_data: Vec::new(),
            },
            FinalChainBlockNumber::GENESIS,
            format!("FINAL_CHAIN_EXECUTION_INVALID_PBFT: {error:#}"),
        ),
    }
}

/// Aborts a FinalChain execution session without touching storage.
///
/// Aborted sessions always report a terminal complete action and reject future
/// commits through `commit_final_chain_execution_session`.
pub fn abort_final_chain_execution_session(
    mut session: FinalChainExecutionSession,
) -> FinalChainExecutionSession {
    session.status = FINAL_CHAIN_EXECUTION_STATUS_ABORTED;
    session.error_code = "FINAL_CHAIN_EXECUTION_ABORTED".to_string();
    session
}

/// Returns the next action that the owner must perform for the session.
///
/// Native-only requests produce a `COMMIT_NATIVE` action. External-EVM-capable
/// requests first ask the retained executor for system transactions, even when
/// the regular transaction list is empty. An empty report with no arbitrary EVM
/// work returns to native commit; a system transaction or arbitrary EVM call
/// produces `EXECUTE_EXTERNAL_EVM` only in the external-EVM mode.
pub fn final_chain_execution_session_next(
    session: &mut FinalChainExecutionSession,
) -> FinalChainExecutionStep {
    match session.status {
        FINAL_CHAIN_EXECUTION_STATUS_REJECTED | FINAL_CHAIN_EXECUTION_STATUS_ABORTED => {
            FinalChainExecutionStep {
                status: session.status,
                action: FINAL_CHAIN_EXECUTION_ACTION_REJECT,
                period: session.block_number,
                error_code: session.error_code.clone(),
                ..Default::default()
            }
        }
        FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM => FinalChainExecutionStep {
            status: session.status,
            action: FINAL_CHAIN_EXECUTION_ACTION_EXECUTE_EXTERNAL_EVM,
            period: session.block_number,
            external_evm_transaction_count: session
                .evm_request
                .as_ref()
                .map(|request| count_external_evm_transactions(&request.transactions))
                .unwrap_or_default(),
            evm_request: session.evm_request.clone().unwrap_or_default(),
            error_code: session.error_code.clone(),
            ..Default::default()
        },
        FINAL_CHAIN_EXECUTION_STATUS_WAITING_SYSTEM_TRANSACTIONS => FinalChainExecutionStep {
            status: session.status,
            action: FINAL_CHAIN_EXECUTION_ACTION_PROVIDE_SYSTEM_TRANSACTIONS,
            period: session.block_number,
            external_evm_transaction_count: count_external_evm_transactions(
                &classify_ordered_execution_transactions(&session.request.transactions)
                    .expect("session constructor validates transaction positions"),
            ),
            system_transaction_request: session
                .system_transaction_request
                .clone()
                .unwrap_or_default(),
            error_code: session.error_code.clone(),
            ..Default::default()
        },
        FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_REWARDS => FinalChainExecutionStep {
            status: session.status,
            action: FINAL_CHAIN_EXECUTION_ACTION_DISTRIBUTE_EXTERNAL_EVM_REWARDS,
            period: session.block_number,
            evm_rewards_request: session.rewards_request.clone().unwrap_or_default(),
            error_code: session.error_code.clone(),
            ..Default::default()
        },
        FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_PUBLICATION => FinalChainExecutionStep {
            status: session.status,
            action: FINAL_CHAIN_EXECUTION_ACTION_PLAN_EXTERNAL_EVM_PUBLICATION,
            period: session.block_number,
            error_code: session.error_code.clone(),
            ..Default::default()
        },
        FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_LIFECYCLE => FinalChainExecutionStep {
            status: session.status,
            action: FINAL_CHAIN_EXECUTION_ACTION_REQUEST_EXTERNAL_EVM_STATE_COMMIT,
            period: session.block_number,
            error_code: session.error_code.clone(),
            ..Default::default()
        },
        FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_STATE_COMMIT => FinalChainExecutionStep {
            status: session.status,
            action: FINAL_CHAIN_EXECUTION_ACTION_REPORT_EXTERNAL_EVM_LIFECYCLE,
            period: session.block_number,
            error_code: session.error_code.clone(),
            ..Default::default()
        },
        FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_STORAGE_PUBLICATION => {
            FinalChainExecutionStep {
                status: session.status,
                action: FINAL_CHAIN_EXECUTION_ACTION_PUBLISH_EXTERNAL_EVM_STORAGE,
                period: session.block_number,
                error_code: session.error_code.clone(),
                ..Default::default()
            }
        }
        FINAL_CHAIN_EXECUTION_STATUS_COMPLETE => FinalChainExecutionStep {
            status: FINAL_CHAIN_EXECUTION_STATUS_COMPLETE,
            action: FINAL_CHAIN_EXECUTION_ACTION_COMPLETE,
            period: session.block_number,
            ..Default::default()
        },
        _ => {
            if let Some(system_request) = session.system_transaction_request.clone() {
                session.status = FINAL_CHAIN_EXECUTION_STATUS_WAITING_SYSTEM_TRANSACTIONS;
                let external_evm_transaction_count = count_external_evm_transactions(
                    &classify_ordered_execution_transactions(&session.request.transactions)
                        .expect("session constructor validates transaction positions"),
                );
                FinalChainExecutionStep {
                    status: session.status,
                    action: FINAL_CHAIN_EXECUTION_ACTION_PROVIDE_SYSTEM_TRANSACTIONS,
                    period: session.block_number,
                    external_evm_transaction_count,
                    system_transaction_request: system_request,
                    error_code: String::new(),
                    ..Default::default()
                }
            } else if let Some(evm_request) = session.evm_request.clone() {
                session.status = FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM;
                let external_evm_transaction_count =
                    count_external_evm_transactions(&evm_request.transactions);
                FinalChainExecutionStep {
                    status: session.status,
                    action: FINAL_CHAIN_EXECUTION_ACTION_EXECUTE_EXTERNAL_EVM,
                    period: session.block_number,
                    external_evm_transaction_count,
                    evm_request,
                    error_code: String::new(),
                    ..Default::default()
                }
            } else {
                FinalChainExecutionStep {
                    status: FINAL_CHAIN_EXECUTION_STATUS_READY,
                    action: FINAL_CHAIN_EXECUTION_ACTION_COMMIT_NATIVE,
                    period: session.block_number,
                    external_evm_transaction_count: 0,
                    evm_request: FinalChainEvmExecutionRequest::default(),
                    error_code: String::new(),
                    ..Default::default()
                }
            }
        }
    }
}

/// Plans bridge-contract system transaction RLPs for external-EVM execution.
///
/// The planner consumes only typed facts supplied by the C++ `StateAPI`
/// boundary. It emits no transaction unless the period is a pillar block, the
/// configured bridge contract exists with code, and the C++ dry-run result says
/// the epoch should finalize. The happy path emits one unsigned legacy system
/// transaction that calls `finalizeEpoch()` on the bridge contract.
pub fn plan_external_evm_system_transactions(
    fact: FinalChainSystemTransactionPlanFact,
) -> Result<FinalChainSystemTransactionPlan, anyhow::Error> {
    if fact.bridge_contract_has_code && !fact.bridge_contract_found {
        anyhow::bail!("bridge contract cannot have code when it was not found");
    }
    if !fact.is_pillar_block_period
        || !fact.bridge_contract_found
        || !fact.bridge_contract_has_code
        || !fact.should_finalize_epoch
    {
        return Ok(FinalChainSystemTransactionPlan {
            request_id: fact.request_id,
            period: fact.period,
            transactions: Vec::new(),
        });
    }

    let system_nonce_bytes = fact.system_account_nonce.to_bytes();
    if system_nonce_bytes.len() > 32 {
        anyhow::bail!("FINAL_CHAIN_SYSTEM_NONCE_EXCEEDS_U256");
    }

    let transaction = encode_legacy_system_transaction(&LegacySystemTransactionInput {
        nonce: U256::from_big_endian(&system_nonce_bytes),
        value: U256::zero(),
        gas_price: U256::zero(),
        gas: fact.block_gas_limit.as_u64(),
        data: solidity_no_arg_call("finalizeEpoch()"),
        receiver: Some(H160::from(fact.bridge_contract_address)),
        chain_id: 0,
    });

    Ok(FinalChainSystemTransactionPlan {
        request_id: fact.request_id,
        period: fact.period,
        transactions: vec![transaction],
    })
}

/// Validates Rust-planned system transaction RLPs and constructs the
/// external EVM request over regular plus appended system transactions.
///
/// The function does not execute or persist system transactions. It decodes
/// canonical RLP bytes with the fixed Taraxa system sender, rejects malformed
/// reports, and includes the resulting facts in the EVM request identity so
/// stale executor reports cannot be replayed across different system payloads.
pub fn final_chain_execution_session_report_system_transactions(
    session: &mut FinalChainExecutionSession,
    report: FinalChainSystemTransactionReport,
) -> FinalChainExecutionStep {
    let system_transaction_count = report.transactions.len();
    final_chain_execution_session_report_system_transactions_with_count(
        session,
        report,
        system_transaction_count,
    )
}

fn final_chain_execution_session_report_system_transactions_with_count(
    session: &mut FinalChainExecutionSession,
    report: FinalChainSystemTransactionReport,
    system_transaction_count: usize,
) -> FinalChainExecutionStep {
    if session.status != FINAL_CHAIN_EXECUTION_STATUS_WAITING_SYSTEM_TRANSACTIONS {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_SYSTEM_TRANSACTIONS_UNEXPECTED".to_string();
        return final_chain_execution_session_next(session);
    }
    let Some(request) = session.system_transaction_request.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_SYSTEM_TRANSACTIONS_WITHOUT_REQUEST".to_string();
        return final_chain_execution_session_next(session);
    };
    if report.request_id != request.request_id {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_SYSTEM_TRANSACTIONS_REQUEST_ID_MISMATCH".to_string();
        return final_chain_execution_session_next(session);
    }
    if report.period != session.block_number {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_SYSTEM_TRANSACTIONS_PERIOD_MISMATCH".to_string();
        return final_chain_execution_session_next(session);
    }

    if let Err(error_code) = validate_combined_transaction_count(
        session.request.transactions.len(),
        system_transaction_count,
    ) {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = error_code.to_string();
        return final_chain_execution_session_next(session);
    }
    let regular_transactions =
        classify_ordered_execution_transactions(&session.request.transactions)
            .expect("session constructor validates transaction positions");
    let system_transactions =
        match decode_system_transaction_inputs(regular_transactions.len(), &report.transactions) {
            Ok(transactions) => transactions,
            Err(error) => {
                session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
                session.error_code = format!("FINAL_CHAIN_SYSTEM_TRANSACTIONS_INVALID: {error:#}");
                return final_chain_execution_session_next(session);
            }
        };
    let mut all_transactions = regular_transactions;
    all_transactions.extend(system_transactions.clone());
    let evm_request = FinalChainEvmExecutionRequest {
        request_id: execution_request_id(
            session.block_number,
            &session.metadata,
            session.request.block_gas_limit,
            FinalChainExternalEvmCommittedStateDescriptor::default(),
            &all_transactions,
        ),
        period: session.block_number,
        prior_state: FinalChainExternalEvmCommittedStateDescriptor::default(),
        concrete_marker_rlp: Vec::new(),
        concrete_plan_hash: [0; 32],
        transactions_hash: [0; 32],
        rewards_hash: [0; 32],
        block_author: session.metadata.author.into(),
        timestamp: session.metadata.timestamp,
        block_gas_limit: session.request.block_gas_limit,
        transactions: all_transactions,
    };
    session.system_transactions = system_transactions;
    session.evm_request = Some(evm_request);
    session.status = FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM;
    session.error_code.clear();
    final_chain_execution_session_next(session)
}

/// Binds the exact concrete prior descriptor into the execution request.
///
/// System-transaction planning is read-only and may happen before this call,
/// but no transaction execution may begin until the request is rebound. The
/// concrete descriptor is part of the deterministic request identity, which
/// prevents an otherwise identical period plan from being replayed on another
/// state root.
fn final_chain_execution_session_bind_external_evm_prior_state(
    session: &mut FinalChainExecutionSession,
    prior_state: FinalChainExternalEvmCommittedStateDescriptor,
) -> Result<FinalChainEvmExecutionRequest, anyhow::Error> {
    ensure!(
        session.status == FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM,
        "FINAL_CHAIN_EVM_PRIOR_STATE_UNEXPECTED"
    );
    ensure!(
        prior_state.state_root != [0; 32],
        "FINAL_CHAIN_EVM_PRIOR_STATE_ROOT_MISSING"
    );
    let request = session
        .evm_request
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_EVM_PRIOR_STATE_WITHOUT_REQUEST"))?;
    request.prior_state = prior_state;
    request.request_id = execution_request_id(
        session.block_number,
        &session.metadata,
        session.request.block_gas_limit,
        prior_state,
        &request.transactions,
    );
    Ok(request.clone())
}

/// Validates an external EVM report against the session's pending request.
///
/// A successful report advances the session to the rewards/state-root boundary
/// instead of committing storage. Failed reports stay terminal rejections.
pub fn final_chain_execution_session_report_evm(
    session: &mut FinalChainExecutionSession,
    report: FinalChainEvmExecutionReport,
) -> FinalChainExecutionStep {
    final_chain_execution_session_report_evm_inner(None, session, report)
}

/// Validates an EVM report and prepares the FinalChain-owned rewards plan used
/// by the production external-execution facade.
pub fn final_chain_execution_session_report_evm_with_final_chain(
    final_chain: &FinalChain,
    session: &mut FinalChainExecutionSession,
    report: FinalChainEvmExecutionReport,
) -> FinalChainExecutionStep {
    final_chain_execution_session_report_evm_inner(Some(final_chain), session, report)
}

fn final_chain_execution_session_report_evm_inner(
    final_chain: Option<&FinalChain>,
    session: &mut FinalChainExecutionSession,
    report: FinalChainEvmExecutionReport,
) -> FinalChainExecutionStep {
    let Some(request) = session.evm_request.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REPORT_WITHOUT_REQUEST".to_string();
        return final_chain_execution_session_next(session);
    };
    if request.request_id != report.request_id {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REPORT_REQUEST_ID_MISMATCH".to_string();
        return final_chain_execution_session_next(session);
    }
    if report.prior_state != request.prior_state {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REPORT_PRIOR_STATE_MISMATCH".to_string();
        return final_chain_execution_session_next(session);
    }
    if report.concrete_marker_rlp != request.concrete_marker_rlp
        || report.concrete_plan_hash != request.concrete_plan_hash
        || report.transactions_hash != request.transactions_hash
        || report.rewards_hash != request.rewards_hash
    {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REPORT_CONCRETE_IDENTITY_MISMATCH".to_string();
        return final_chain_execution_session_next(session);
    }
    if decode_concrete_execution_marker(&report.concrete_marker_rlp).is_err()
        || concrete_state_bytes_digest(&report.concrete_marker_rlp) == [0; 32]
    {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REPORT_CONCRETE_MARKER_INVALID".to_string();
        return final_chain_execution_session_next(session);
    }
    if report.post_transaction_state_root == [0; 32] {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REPORT_POST_TRANSACTION_ROOT_MISSING".to_string();
        return final_chain_execution_session_next(session);
    }
    if request.transactions.len() != report.results.len() {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REPORT_RESULT_COUNT_MISMATCH".to_string();
        return final_chain_execution_session_next(session);
    }
    if report.status != FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REPORT_REJECTED".to_string();
        return final_chain_execution_session_next(session);
    }
    let mut cumulative_gas_used = FinalChainGas::ZERO;
    for (expected, actual) in request.transactions.iter().zip(report.results.iter()) {
        if expected.position != actual.position || expected.hash != actual.hash {
            session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
            session.error_code = "FINAL_CHAIN_EVM_REPORT_TRANSACTION_MISMATCH".to_string();
            return final_chain_execution_session_next(session);
        }
        if actual.status > 1 {
            session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
            session.error_code = "FINAL_CHAIN_EVM_REPORT_TRANSACTION_STATUS_INVALID".to_string();
            return final_chain_execution_session_next(session);
        }
        let has_error = !actual.code_error.is_empty() || !actual.consensus_error.is_empty();
        if (actual.status == 1) == has_error {
            session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
            session.error_code =
                "FINAL_CHAIN_EVM_REPORT_TRANSACTION_STATUS_ERROR_MISMATCH".to_string();
            return final_chain_execution_session_next(session);
        }
        cumulative_gas_used = match cumulative_gas_used.checked_add(actual.gas_used) {
            Some(value) => value,
            None => {
                session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
                session.error_code = "FINAL_CHAIN_EVM_REPORT_GAS_OVERFLOW".to_string();
                return final_chain_execution_session_next(session);
            }
        };
        if actual.cumulative_gas_used != cumulative_gas_used {
            session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
            session.error_code = "FINAL_CHAIN_EVM_REPORT_CUMULATIVE_GAS_MISMATCH".to_string();
            return final_chain_execution_session_next(session);
        }
        let receipt = rlp::Rlp::new(&actual.receipt_rlp);
        if !receipt.is_list() || receipt.item_count().is_err() {
            session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
            session.error_code = "FINAL_CHAIN_EVM_REPORT_RECEIPT_RLP_MALFORMED".to_string();
            return final_chain_execution_session_next(session);
        }
    }
    if report.cumulative_gas_used != cumulative_gas_used {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REPORT_TOTAL_GAS_MISMATCH".to_string();
        return final_chain_execution_session_next(session);
    }
    let prepared_plan = if let Some(final_chain) = final_chain {
        let rewards_fact = match build_external_evm_rewards_fact(&session.request, request, &report)
        {
            Ok(fact) => fact,
            Err(error) => {
                session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
                session.error_code = format!("FINAL_CHAIN_EVM_REWARDS_FACT_INVALID: {error:#}");
                return final_chain_execution_session_next(session);
            }
        };
        match final_chain.plan_external_evm_rewards_stats(request.request_id, rewards_fact) {
            Ok(plan) => Some(plan),
            Err(error) => {
                session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
                session.error_code = format!("FINAL_CHAIN_EVM_REWARDS_PLAN_INVALID: {error:#}");
                return final_chain_execution_session_next(session);
            }
        }
    } else {
        None
    };
    let rewards_request =
        match build_external_evm_rewards_request(&session.request, request, &report) {
            Ok(request) => request,
            Err(error) => {
                session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
                session.error_code = format!("FINAL_CHAIN_EVM_REWARDS_REQUEST_INVALID: {error:#}");
                return final_chain_execution_session_next(session);
            }
        };
    let mut rewards_request = rewards_request;
    if let Some(plan) = prepared_plan.as_ref() {
        rewards_request.distribution_stats = plan.distribution_stats.clone();
    }
    session.report = Some(report);
    session.rewards_request = Some(rewards_request);
    session.prepared_rewards_stats_plan = prepared_plan;
    session.status = FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_REWARDS;
    session.error_code.clear();
    final_chain_execution_session_next(session)
}

/// Validates external EVM rewards/state-root facts and builds a Rust commit
/// plan without mutating FinalChain storage.
///
/// The returned plan contains exact lifecycle roots, header roots, blooms,
/// receipt payloads, gas, and execution counters for the later Rust-owned
/// publication batch. The function intentionally does not call `StateAPI`,
/// write storage, or mark the session complete.
pub fn final_chain_execution_session_plan_external_evm_commit(
    session: &mut FinalChainExecutionSession,
    rewards_report: FinalChainEvmRewardsReport,
) -> FinalChainExternalEvmCommitPlan {
    if session.status != FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_REWARDS {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_UNEXPECTED".to_string();
        return rejected_external_evm_commit_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    let Some(evm_request) = session.evm_request.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_WITHOUT_REQUEST".to_string();
        return rejected_external_evm_commit_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    };
    let Some(evm_report) = session.report.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_WITHOUT_EVM_REPORT".to_string();
        return rejected_external_evm_commit_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    };
    if rewards_report.request_id != evm_request.request_id {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_REQUEST_ID_MISMATCH".to_string();
        return rejected_external_evm_commit_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if rewards_report.period != session.block_number {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_PERIOD_MISMATCH".to_string();
        return rejected_external_evm_commit_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if rewards_report.status != FINAL_CHAIN_EVM_REWARDS_REPORT_STATUS_SUCCESS {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_REJECTED".to_string();
        return rejected_external_evm_commit_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if rewards_report.prior_state != evm_request.prior_state {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_PRIOR_STATE_MISMATCH".to_string();
        return rejected_external_evm_commit_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if rewards_report.post_transaction_state_root != evm_report.post_transaction_state_root {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code =
            "FINAL_CHAIN_EVM_REWARDS_REPORT_POST_TRANSACTION_ROOT_MISMATCH".to_string();
        return rejected_external_evm_commit_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if rewards_report.concrete_marker_rlp != evm_request.concrete_marker_rlp
        || rewards_report.concrete_plan_hash != evm_request.concrete_plan_hash
        || rewards_report.transactions_hash != evm_request.transactions_hash
        || rewards_report.rewards_hash != evm_request.rewards_hash
    {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code =
            "FINAL_CHAIN_EVM_REWARDS_REPORT_CONCRETE_IDENTITY_MISMATCH".to_string();
        return rejected_external_evm_commit_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if rewards_report.concrete_projection_hash
        != concrete_state_bytes_digest(&rewards_report.concrete_projection_rlp)
    {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_PROJECTION_HASH_MISMATCH".to_string();
        return rejected_external_evm_commit_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if rewards_report.post_rewards_state_root == [0; 32] {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_POST_REWARDS_ROOT_MISSING".to_string();
        return rejected_external_evm_commit_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if rewards_report.total_reward.len() > 32 {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_TOTAL_REWARD_OVERSIZED".to_string();
        return rejected_external_evm_commit_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if let Err(error) = validate_concrete_execution_results(
        &rewards_report.concrete_projection_rlp,
        &evm_report.results,
    ) {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = format!("FINAL_CHAIN_EVM_CONCRETE_RESULT_MISMATCH: {error:#}");
        return rejected_external_evm_commit_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    match build_external_evm_commit_plan(
        &session.request,
        &session.metadata,
        session.block_number,
        evm_request,
        evm_report,
        &rewards_report,
    ) {
        Ok(plan) => {
            session.external_evm_commit_plan = Some(plan.clone());
            session.status = FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_PUBLICATION;
            session.error_code.clear();
            plan
        }
        Err(error) => {
            session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
            session.error_code = format!("FINAL_CHAIN_EVM_COMMIT_PLAN_INVALID: {error:#}");
            rejected_external_evm_commit_plan(
                session.block_number,
                &session.metadata,
                session.error_code.clone(),
            )
        }
    }
}

/// Builds the external-EVM storage-publication facts without mutating storage.
///
/// This materializes header bytes, receipt bytes, transaction index facts,
/// system-transaction hash rows, a deterministic publication plan id, and
/// absolute execution counters. Successful planning advances the session to
/// external EVM lifecycle validation, but still does not publish storage or
/// touch `StateAPI`.
pub fn final_chain_execution_session_plan_external_evm_publication(
    final_chain: &FinalChain,
    session: &mut FinalChainExecutionSession,
) -> FinalChainExternalEvmPublicationPlan {
    if session.status != FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_PUBLICATION {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_PUBLICATION_UNEXPECTED".to_string();
        return rejected_external_evm_publication_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    let Some(evm_request) = session.evm_request.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_PUBLICATION_WITHOUT_REQUEST".to_string();
        return rejected_external_evm_publication_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    };
    let Some(commit_plan) = session.external_evm_commit_plan.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_PUBLICATION_WITHOUT_COMMIT_PLAN".to_string();
        return rejected_external_evm_publication_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    };
    match build_external_evm_publication_plan(
        final_chain,
        &session.request.pbft_block_rlp,
        &session.metadata,
        session.block_number,
        evm_request,
        commit_plan,
    ) {
        Ok(publication) => {
            session.external_evm_publication_plan = Some(publication.clone());
            session.status = FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_LIFECYCLE;
            session.error_code.clear();
            publication
        }
        Err(error) => {
            session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
            session.error_code = format!("FINAL_CHAIN_EVM_PUBLICATION_INVALID: {error:#}");
            rejected_external_evm_publication_plan(
                session.block_number,
                &session.metadata,
                session.error_code.clone(),
            )
        }
    }
}

/// Validates immutable external EVM staged-state facts before C++ commits state.
///
/// The returned ready intent allows the caller to invoke
/// `StateAPI::transition_state_commit`, but it is not publish-safe. Rust stores
/// the accepted intent and requires a later lifecycle report with the same
/// facts before returning a final `READY_TO_PUBLISH` decision.
pub fn final_chain_execution_session_request_external_evm_state_commit(
    session: &mut FinalChainExecutionSession,
    request: FinalChainExternalEvmStateCommitRequest,
) -> FinalChainExternalEvmStateCommitIntent {
    if session.status != FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_LIFECYCLE {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_STATE_COMMIT_UNEXPECTED".to_string();
        return rejected_external_evm_state_commit_intent(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if let Err(error_code) =
        validate_external_evm_state_commit_facts(session, &request, "FINAL_CHAIN_EVM_STATE_COMMIT")
    {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = error_code;
        return rejected_external_evm_state_commit_intent(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }

    let intent = FinalChainExternalEvmStateCommitIntent {
        request_id: request.request_id,
        plan_id: request.plan_id,
        period: request.period,
        publication_block_hash: request.publication_block_hash,
        prior_state: request.prior_state,
        post_transaction_state_root: request.post_transaction_state_root,
        post_rewards_state_root: request.post_rewards_state_root,
        expected_state_root: request.post_rewards_state_root,
        concrete_marker_rlp: request.concrete_marker_rlp,
        concrete_projection_rlp: request.concrete_projection_rlp,
        concrete_projection_hash: request.concrete_projection_hash,
        concrete_provenance_rlp: request.concrete_provenance_rlp,
        status: FINAL_CHAIN_EVM_STATE_COMMIT_INTENT_READY_TO_COMMIT,
        error_code: String::new(),
    };
    session.external_evm_state_commit_intent = Some(intent.clone());
    session.status = FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_STATE_COMMIT;
    session.error_code.clear();
    intent
}

/// Prepares the external-EVM publication state-commit lane in one step.
///
/// The helper owns publication planning, optional rewards-stat/proposal-period plan
/// attachment, state-commit approval, and pending-publication marker persistence.
/// It returns the Rust state-commit intent that must be reported after the C++
/// side confirms `StateAPI::transition_state_commit()`.
pub fn final_chain_execution_session_prepare_external_evm_state_commit(
    final_chain: &FinalChain,
    session: &mut FinalChainExecutionSession,
    proposal_period_update: FinalChainProposalPeriodDagLevelUpdate,
) -> Result<FinalChainExternalEvmStateCommitIntent, anyhow::Error> {
    let prepared_rewards_stats_plan = session_external_evm_rewards_stats_plan(session)?.clone();
    let rewards_stats_update = final_chain
        .validate_external_evm_rewards_stats_plan(&prepared_rewards_stats_plan)
        .map_err(|error| anyhow::anyhow!("FINAL_CHAIN_EVM_REWARDS_STATS_STALE: {error:#}"))?;
    let publication_plan =
        final_chain_execution_session_plan_external_evm_publication(final_chain, session);
    if !publication_plan.error_code.is_empty() {
        return Err(anyhow::anyhow!(
            "FINAL_CHAIN_EVM_PUBLICATION_PLAN_PREPARE_FAILED: {}",
            publication_plan.error_code
        ));
    }

    let publication_plan = final_chain_execution_session_attach_external_evm_rewards_stats(
        session,
        rewards_stats_update,
    );
    if !publication_plan.error_code.is_empty() {
        return Err(anyhow::anyhow!(
            "FINAL_CHAIN_EVM_PUBLICATION_REWARDS_STATS_PREPARE_FAILED: {}",
            publication_plan.error_code
        ));
    }

    let publication_plan =
        final_chain_execution_session_attach_external_evm_proposal_period_dag_level(
            session,
            proposal_period_update,
        );
    if !publication_plan.error_code.is_empty() {
        return Err(anyhow::anyhow!(
            "FINAL_CHAIN_EVM_PROPOSAL_PERIOD_MAPPING_PREPARE_FAILED: {}",
            publication_plan.error_code
        ));
    }

    let evm_request = session
        .evm_request
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_CONCRETE_PROJECTION_WITHOUT_REQUEST"))?;
    let marker = decode_concrete_execution_marker(&evm_request.concrete_marker_rlp)
        .context("FINAL_CHAIN_CONCRETE_MARKER_INVALID")?;
    let projection = decode_concrete_state_projection(&publication_plan.concrete_projection_rlp)
        .context("FINAL_CHAIN_CONCRETE_PROJECTION_INVALID")?;
    ensure!(
        publication_plan.concrete_projection_hash
            == concrete_state_bytes_digest(&publication_plan.concrete_projection_rlp),
        "FINAL_CHAIN_CONCRETE_PROJECTION_HASH_MISMATCH"
    );
    ensure!(
        projection.identity == marker.identity
            && projection.generation == marker.generation
            && projection.plan_hash == marker.plan_hash
            && projection.prior_state == marker.prior_state
            && projection.post_transaction_state.period == marker.period
            && projection.post_transaction_state.root
                == commit_plan_post_transaction_root(session)?
            && projection.post_rewards_state.period == marker.period
            && projection.post_rewards_state.root == commit_plan_post_rewards_root(session)?,
        "FINAL_CHAIN_CONCRETE_PROJECTION_LINEAGE_MISMATCH"
    );
    ensure!(
        projection.transaction_effects.len() == evm_request.transactions.len(),
        "FINAL_CHAIN_CONCRETE_PROJECTION_TRANSACTION_COUNT_MISMATCH"
    );
    ensure!(
        projection
            .transaction_effects
            .last()
            .is_none_or(|effect| effect.intermediate_state == projection.post_transaction_state),
        "FINAL_CHAIN_CONCRETE_PROJECTION_FINAL_TRANSACTION_ROOT_MISMATCH"
    );
    let expected_rewards_input =
        encode_concrete_rewards_input(&prepared_rewards_stats_plan.distribution_stats);
    ensure!(
        projection.rewards_input == expected_rewards_input,
        "FINAL_CHAIN_CONCRETE_PROJECTION_REWARDS_INPUT_MISMATCH"
    );
    let reported_total_reward = session
        .external_evm_commit_plan
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_CONCRETE_PROJECTION_WITHOUT_COMMIT_PLAN"))?
        .total_reward
        .clone();
    let (dpos_snapshot_rlp, account_snapshot_rlp) = final_chain.external_evm_concrete_projection(
        session.block_number,
        evm_request.block_author,
        &evm_request.transactions,
        &projection,
        &prepared_rewards_stats_plan,
        &reported_total_reward,
    )?;
    let concrete_provenance_rlp =
        encode_concrete_state_provenance(&FinalChainConcreteStateProvenance {
            identity: projection.identity,
            generation: projection.generation,
            plan_hash: projection.plan_hash,
            committed_state: projection.post_rewards_state,
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            projection_hash: publication_plan.concrete_projection_hash,
            catalog_hash: projection.catalog_hash,
        });
    let publication_plan = session
        .external_evm_publication_plan
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_EVM_DPOS_PROJECTION_WITHOUT_PUBLICATION"))?;
    publication_plan.dpos_snapshot_rlp = dpos_snapshot_rlp;
    publication_plan.account_snapshot_rlp = account_snapshot_rlp;
    publication_plan.concrete_provenance_rlp = concrete_provenance_rlp.clone();
    publication_plan.plan_id = final_chain_external_evm_publication_plan_id(publication_plan);
    if let Some(commit_plan) = session.external_evm_commit_plan.as_mut() {
        commit_plan.concrete_provenance_rlp = concrete_provenance_rlp;
    }
    let publication_plan = publication_plan.clone();

    let state_commit_request_step = final_chain_execution_session_next(session);
    if state_commit_request_step.action
        != FINAL_CHAIN_EXECUTION_ACTION_REQUEST_EXTERNAL_EVM_STATE_COMMIT
    {
        return Err(anyhow::anyhow!(
            "FINAL_CHAIN_EVM_STATE_COMMIT_UNEXPECTED_AFTER_PUBLICATION_PLAN: {}",
            state_commit_request_step.action
        ));
    }

    let commit_plan = session
        .external_evm_commit_plan
        .clone()
        .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_EVM_STATE_COMMIT_WITHOUT_COMMIT_PLAN"))?;

    let state_commit_request = FinalChainExternalEvmStateCommitRequest {
        request_id: publication_plan.request_id,
        plan_id: publication_plan.plan_id,
        period: publication_plan.period,
        prior_state: commit_plan.prior_state,
        post_transaction_state_root: commit_plan.post_transaction_state_root,
        post_rewards_state_root: commit_plan.post_rewards_state_root,
        publication_block_hash: publication_plan.block_hash,
        concrete_marker_rlp: publication_plan.concrete_marker_rlp.clone(),
        concrete_projection_rlp: publication_plan.concrete_projection_rlp.clone(),
        concrete_projection_hash: publication_plan.concrete_projection_hash,
        concrete_provenance_rlp: publication_plan.concrete_provenance_rlp.clone(),
    };

    let intent = final_chain_execution_session_request_external_evm_state_commit(
        session,
        state_commit_request,
    );
    if intent.status != FINAL_CHAIN_EVM_STATE_COMMIT_INTENT_READY_TO_COMMIT {
        return Err(anyhow::anyhow!("{}", intent.error_code));
    }

    let pending_publication_report =
        final_chain_execution_session_persist_external_evm_pending_publication(
            final_chain,
            session,
        )?;
    if pending_publication_report.status != FINAL_CHAIN_EVM_PUBLICATION_STATUS_APPLIED
        || !pending_publication_report.error_code.is_empty()
    {
        return Err(anyhow::anyhow!(
            "FINAL_CHAIN_EVM_PENDING_PUBLICATION_PREPARE_FAILED: {}",
            pending_publication_report.error_code
        ));
    }

    Ok(intent)
}

fn session_external_evm_rewards_stats_plan(
    session: &FinalChainExecutionSession,
) -> Result<&FinalChainPreparedExternalEvmRewardsStatsPlan, anyhow::Error> {
    let plan = session
        .prepared_rewards_stats_plan
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_EVM_REWARDS_STATS_PLAN_MISSING"))?;
    let evm_request = session
        .evm_request
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_EVM_REWARDS_STATS_REQUEST_MISSING"))?;
    if plan.request_id != evm_request.request_id
        || plan.period != evm_request.period
        || plan.period != session.block_number
    {
        anyhow::bail!("FINAL_CHAIN_EVM_REWARDS_STATS_SESSION_MISMATCH");
    }
    Ok(plan)
}

/// Validates external EVM staged-state lifecycle facts and returns the final
/// Rust commit decision.
///
/// A committed lifecycle report must follow a ready state-commit intent and
/// match the exact EVM execution root, post-rewards root, and publication block
/// hash derived by Rust. Ready publication decisions remain non-mutating and
/// advance the session to an explicit storage-publication action; callers must
/// still drive that action before FinalChain storage is written.
pub fn final_chain_execution_session_report_external_evm_lifecycle(
    session: &mut FinalChainExecutionSession,
    report: FinalChainExternalEvmLifecycleReport,
) -> FinalChainExternalEvmCommitDecision {
    if session.status != FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_STATE_COMMIT {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_UNEXPECTED".to_string();
        return rejected_external_evm_commit_decision(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    let Some(intent) = session.external_evm_state_commit_intent.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_WITHOUT_STATE_COMMIT_INTENT".to_string();
        return rejected_external_evm_commit_decision(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    };
    if report.request_id != intent.request_id {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_INTENT_REQUEST_ID_MISMATCH".to_string();
        return rejected_external_evm_commit_decision(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if report.plan_id != intent.plan_id {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_INTENT_PLAN_ID_MISMATCH".to_string();
        return rejected_external_evm_commit_decision(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if report.period != intent.period {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_INTENT_PERIOD_MISMATCH".to_string();
        return rejected_external_evm_commit_decision(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if report.publication_block_hash != intent.publication_block_hash {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_INTENT_BLOCK_HASH_MISMATCH".to_string();
        return rejected_external_evm_commit_decision(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    let report_facts = FinalChainExternalEvmStateCommitRequest {
        request_id: report.request_id,
        plan_id: report.plan_id,
        period: report.period,
        prior_state: report.prior_state,
        post_transaction_state_root: report.post_transaction_state_root,
        post_rewards_state_root: report.post_rewards_state_root,
        publication_block_hash: report.publication_block_hash,
        concrete_marker_rlp: intent.concrete_marker_rlp.clone(),
        concrete_projection_rlp: intent.concrete_projection_rlp.clone(),
        concrete_projection_hash: intent.concrete_projection_hash,
        concrete_provenance_rlp: intent.concrete_provenance_rlp.clone(),
    };
    if let Err(error_code) = validate_external_evm_state_commit_facts(
        session,
        &report_facts,
        "FINAL_CHAIN_EVM_LIFECYCLE",
    ) {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = error_code;
        return rejected_external_evm_commit_decision(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    let expected_committed_state = FinalChainExternalEvmCommittedStateDescriptor {
        period: intent.period,
        state_root: intent.post_rewards_state_root,
    };
    match report.status {
        FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED
            if report.committed_state != Some(expected_committed_state) =>
        {
            session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
            session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_COMMITTED_DESCRIPTOR_MISMATCH".into();
            return rejected_external_evm_commit_decision(
                session.block_number,
                &session.metadata,
                session.error_code.clone(),
            );
        }
        FINAL_CHAIN_EVM_LIFECYCLE_STATUS_DISCARDED
            if report.committed_state != Some(intent.prior_state) =>
        {
            session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
            session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_DISCARDED_DESCRIPTOR_MISMATCH".into();
            return rejected_external_evm_commit_decision(
                session.block_number,
                &session.metadata,
                session.error_code.clone(),
            );
        }
        _ => {}
    }
    let has_error = !report.error_code.is_empty();
    if report.status == FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED && has_error {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_COMMITTED_WITH_ERROR".to_string();
        return rejected_external_evm_commit_decision(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if report.status != FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = match (report.status, report.error_code.is_empty()) {
            (FINAL_CHAIN_EVM_LIFECYCLE_STATUS_DISCARDED, true) => {
                "FINAL_CHAIN_EVM_LIFECYCLE_DISCARDED".to_string()
            }
            (FINAL_CHAIN_EVM_LIFECYCLE_STATUS_DISCARDED, false) => {
                format!("FINAL_CHAIN_EVM_LIFECYCLE_DISCARDED: {}", report.error_code)
            }
            (FINAL_CHAIN_EVM_LIFECYCLE_STATUS_REJECTED, true) => {
                "FINAL_CHAIN_EVM_LIFECYCLE_REJECTED".to_string()
            }
            (FINAL_CHAIN_EVM_LIFECYCLE_STATUS_REJECTED, false) => {
                format!("FINAL_CHAIN_EVM_LIFECYCLE_REJECTED: {}", report.error_code)
            }
            (_, true) => "FINAL_CHAIN_EVM_LIFECYCLE_STATUS_INVALID".to_string(),
            (_, false) => format!(
                "FINAL_CHAIN_EVM_LIFECYCLE_STATUS_INVALID: {}",
                report.error_code
            ),
        };
        return rejected_external_evm_commit_decision(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }

    let decision = FinalChainExternalEvmCommitDecision {
        request_id: intent.request_id,
        plan_id: intent.plan_id,
        decision_id: final_chain_external_evm_commit_decision_id(
            intent.request_id,
            intent.plan_id,
            intent.period,
            intent.publication_block_hash,
        ),
        period: intent.period,
        publication_block_hash: intent.publication_block_hash,
        status: FINAL_CHAIN_EVM_COMMIT_DECISION_READY_TO_PUBLISH,
        error_code: String::new(),
    };
    session.external_evm_commit_decision = Some(decision.clone());
    session.status = FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_STORAGE_PUBLICATION;
    session.error_code.clear();
    decision
}

/// Reports the external EVM state-commit result through a Rust-owned lifecycle
/// adapter.
///
/// The caller echoes the accepted identity/transition facts and supplies the
/// observed committed descriptor. Rust compares them with the session-owned
/// intent and commit plan, then advances only an exactly committed outcome to
/// publication. An exactly correlated discarded outcome clears the marker;
/// mismatched or rejected outcomes keep it durable so restart recovery can
/// arbitrate the ambiguous boundary.
pub fn final_chain_execution_session_report_external_evm_state_commit_result(
    final_chain: &FinalChain,
    session: &mut FinalChainExecutionSession,
    result: FinalChainExternalEvmStateCommitResult,
) -> Result<FinalChainExternalEvmCommitDecision, anyhow::Error> {
    if session.status != FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_STATE_COMMIT {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_STATE_COMMIT_RESULT_UNEXPECTED".to_string();
        return Ok(rejected_external_evm_commit_decision(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        ));
    }
    let Some(intent) = session.external_evm_state_commit_intent.clone() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code =
            "FINAL_CHAIN_EVM_STATE_COMMIT_RESULT_WITHOUT_STATE_COMMIT_INTENT".to_string();
        return Ok(rejected_external_evm_commit_decision(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        ));
    };
    let Some(commit_plan) = session.external_evm_commit_plan.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_STATE_COMMIT_RESULT_WITHOUT_COMMIT_PLAN".to_string();
        return Ok(rejected_external_evm_commit_decision(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        ));
    };

    let status = result.status;
    let validation = validate_external_evm_state_commit_result_facts(&intent, commit_plan, &result);
    if let Err(error_code) = validation {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = error_code.to_string();
        return Ok(rejected_external_evm_commit_decision(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        ));
    }
    let correlated_discard = status == FINAL_CHAIN_EVM_LIFECYCLE_STATUS_DISCARDED;
    let decision = final_chain_execution_session_report_external_evm_lifecycle(
        session,
        FinalChainExternalEvmLifecycleReport {
            request_id: result.request_id,
            plan_id: result.plan_id,
            period: result.period,
            prior_state: result.prior_state,
            post_transaction_state_root: result.post_transaction_state_root,
            post_rewards_state_root: result.post_rewards_state_root,
            publication_block_hash: result.publication_block_hash,
            committed_state: result.committed_state,
            status,
            error_code: result.error_code,
        },
    );
    if correlated_discard && decision.status == FINAL_CHAIN_EVM_COMMIT_DECISION_REJECTED {
        final_chain.clear_external_evm_pending_publication_marker()?;
    }
    Ok(decision)
}

/// Publishes the session-owned external-EVM FinalChain storage batch.
///
/// The session must first expose
/// `FINAL_CHAIN_EXECUTION_ACTION_PUBLISH_EXTERNAL_EVM_STORAGE` after successful
/// lifecycle validation. Rust uses the plan and ready decision stored in the
/// session, calls the lower-level FinalChain storage primitive, and then marks
/// the session complete for applied/already-applied reports or rejected for any
/// rejected publication report. EVM execution, rewards distribution, and
/// `StateAPI` lifecycle ownership remain outside Rust.
pub fn final_chain_execution_session_publish_external_evm_publication(
    final_chain: &FinalChain,
    session: &mut FinalChainExecutionSession,
) -> Result<FinalChainExternalEvmPublicationReport, anyhow::Error> {
    if session.status != FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_STORAGE_PUBLICATION {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_STORAGE_PUBLICATION_UNEXPECTED".to_string();
        return Ok(rejected_session_external_evm_publication_report(
            session,
            session.error_code.clone(),
        ));
    }
    let Some(publication_plan) = session.external_evm_publication_plan.clone() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_STORAGE_PUBLICATION_WITHOUT_PLAN".to_string();
        return Ok(rejected_session_external_evm_publication_report(
            session,
            session.error_code.clone(),
        ));
    };
    let Some(decision) = session.external_evm_commit_decision.clone() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_STORAGE_PUBLICATION_WITHOUT_DECISION".to_string();
        return Ok(rejected_session_external_evm_publication_report(
            session,
            session.error_code.clone(),
        ));
    };

    let report = final_chain.publish_external_evm_publication(publication_plan, decision)?;
    match report.status {
        FINAL_CHAIN_EVM_PUBLICATION_STATUS_APPLIED
        | FINAL_CHAIN_EVM_PUBLICATION_STATUS_ALREADY_APPLIED => {
            if report.error_code.is_empty() {
                session.status = FINAL_CHAIN_EXECUTION_STATUS_COMPLETE;
                session.error_code.clear();
            } else {
                session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
                session.error_code = format!(
                    "FINAL_CHAIN_EVM_STORAGE_PUBLICATION_APPLIED_WITH_ERROR: {}",
                    report.error_code
                );
            }
        }
        FINAL_CHAIN_EVM_PUBLICATION_STATUS_REJECTED => {
            session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
            session.error_code = if report.error_code.is_empty() {
                "FINAL_CHAIN_EVM_STORAGE_PUBLICATION_REJECTED".to_string()
            } else {
                report.error_code.clone()
            };
        }
        _ => {
            session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
            session.error_code = if report.error_code.is_empty() {
                "FINAL_CHAIN_EVM_STORAGE_PUBLICATION_STATUS_INVALID".to_string()
            } else {
                format!(
                    "FINAL_CHAIN_EVM_STORAGE_PUBLICATION_STATUS_INVALID: {}",
                    report.error_code
                )
            };
        }
    }
    Ok(report)
}

/// Persists the session-owned external-EVM pending publication recovery marker.
///
/// Callers must invoke this after Rust returns a ready state-commit intent and
/// before calling `StateAPI::transition_state_commit()`. The marker does not
/// authorize publication by itself; restart recovery still requires the C++
/// StateAPI committed descriptor to match the marker period and post-rewards
/// state root exactly.
pub fn final_chain_execution_session_persist_external_evm_pending_publication(
    final_chain: &FinalChain,
    session: &mut FinalChainExecutionSession,
) -> Result<FinalChainExternalEvmPublicationReport, anyhow::Error> {
    if session.status != FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_STATE_COMMIT {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_PENDING_PUBLICATION_UNEXPECTED".to_string();
        return Ok(rejected_session_external_evm_publication_report(
            session,
            session.error_code.clone(),
        ));
    }
    let Some(publication_plan) = session.external_evm_publication_plan.clone() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_PENDING_PUBLICATION_WITHOUT_PLAN".to_string();
        return Ok(rejected_session_external_evm_publication_report(
            session,
            session.error_code.clone(),
        ));
    };
    let Some(commit_plan) = session.external_evm_commit_plan.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_PENDING_PUBLICATION_WITHOUT_COMMIT_PLAN".to_string();
        return Ok(rejected_session_external_evm_publication_report(
            session,
            session.error_code.clone(),
        ));
    };
    let Some(intent) = session.external_evm_state_commit_intent.clone() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code =
            "FINAL_CHAIN_EVM_PENDING_PUBLICATION_WITHOUT_STATE_COMMIT_INTENT".to_string();
        return Ok(rejected_session_external_evm_publication_report(
            session,
            session.error_code.clone(),
        ));
    };
    let report = final_chain.write_external_evm_pending_publication_marker(
        external_evm_pending_publication_marker(
            publication_plan,
            intent,
            commit_plan.post_transaction_state_root,
            commit_plan.post_rewards_state_root,
        ),
    )?;
    if report.status == FINAL_CHAIN_EVM_PUBLICATION_STATUS_APPLIED && report.error_code.is_empty() {
        session.error_code.clear();
    } else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = if report.error_code.is_empty() {
            "FINAL_CHAIN_EVM_PENDING_PUBLICATION_REJECTED".to_string()
        } else {
            report.error_code.clone()
        };
    }
    Ok(report)
}

/// Attaches rewards-stat cache publication facts to an external-EVM plan.
///
/// The session planner derives all header, receipt, bloom, and transaction
/// publication facts before the C++ executor distributes rewards. The executor
/// then supplies the Rust rewards-stat cache mutation through this helper so
/// the final plan id covers the complete Rust storage batch.
fn final_chain_external_evm_publication_plan_with_rewards_stats(
    mut plan: FinalChainExternalEvmPublicationPlan,
    rewards_stats_update: FinalChainExternalEvmRewardsStatsUpdate,
) -> FinalChainExternalEvmPublicationPlan {
    plan.rewards_stats_update = rewards_stats_update;
    plan.plan_id = final_chain_external_evm_publication_plan_id(&plan);
    plan
}

/// Attaches rewards-stat facts to the session-owned external-EVM publication
/// plan and returns the rehashed plan.
///
/// This must run after publication planning and before lifecycle reporting so
/// lifecycle validation and storage publication agree on the same full plan id.
pub fn final_chain_execution_session_attach_external_evm_rewards_stats(
    session: &mut FinalChainExecutionSession,
    rewards_stats_update: FinalChainExternalEvmRewardsStatsUpdate,
) -> FinalChainExternalEvmPublicationPlan {
    if session.status != FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_LIFECYCLE {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_STATS_UNEXPECTED".to_string();
        return rejected_external_evm_publication_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    let Some(publication_plan) = session.external_evm_publication_plan.take() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_STATS_WITHOUT_PUBLICATION_PLAN".to_string();
        return rejected_external_evm_publication_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    };

    let publication_plan = final_chain_external_evm_publication_plan_with_rewards_stats(
        publication_plan,
        rewards_stats_update,
    );
    session.external_evm_publication_plan = Some(publication_plan.clone());
    publication_plan
}

/// Attaches the optional proposal-period DAG-level mapping to the session plan.
///
/// The C++ boundary supplies this fact from its temporary DAG anchor sidecar,
/// but Rust owns the publication plan identity and the eventual storage batch.
/// This must run before the external EVM state-commit request is derived so
/// lifecycle validation and storage publication cover the mapping row.
pub fn final_chain_execution_session_attach_external_evm_proposal_period_dag_level(
    session: &mut FinalChainExecutionSession,
    update: FinalChainProposalPeriodDagLevelUpdate,
) -> FinalChainExternalEvmPublicationPlan {
    if session.status != FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_LIFECYCLE {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_PROPOSAL_PERIOD_MAPPING_UNEXPECTED".to_string();
        return rejected_external_evm_publication_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    }
    let Some(mut publication_plan) = session.external_evm_publication_plan.take() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code =
            "FINAL_CHAIN_EVM_PROPOSAL_PERIOD_MAPPING_WITHOUT_PUBLICATION_PLAN".to_string();
        return rejected_external_evm_publication_plan(
            session.block_number,
            &session.metadata,
            session.error_code.clone(),
        );
    };

    publication_plan.proposal_period_dag_level_update = update;
    publication_plan.plan_id = final_chain_external_evm_publication_plan_id(&publication_plan);
    session.external_evm_publication_plan = Some(publication_plan.clone());
    publication_plan
}

/// Commits a completed native FinalChain execution session.
///
/// Only explicit native-reference sessions whose next step is `COMMIT_NATIVE`
/// use this path. Production external-enabled sessions publish only after the
/// concrete lifecycle and durable-marker protocol succeeds.
pub fn commit_final_chain_execution_session(
    final_chain: &FinalChain,
    mut session: FinalChainExecutionSession,
) -> Result<FinalChainExecutionCommitReport, anyhow::Error> {
    let step = final_chain_execution_session_next(&mut session);
    if step.action != FINAL_CHAIN_EXECUTION_ACTION_COMMIT_NATIVE {
        return Ok(FinalChainExecutionCommitReport {
            status: FINAL_CHAIN_EXECUTION_STATUS_REJECTED,
            period: session.block_number,
            error_code: if step.error_code.is_empty() {
                "FINAL_CHAIN_EXECUTION_NOT_NATIVE_COMMIT".to_string()
            } else {
                step.error_code
            },
            ..Default::default()
        });
    }

    let FinalChainExecutionRequest {
        pbft_block_rlp,
        transactions,
        finalized_dag_blocks,
        blocks_per_year,
        cert_votes,
        ..
    } = session.request;
    let executed_dag_blocks = finalized_dag_blocks.len() as u64;
    let executed_transactions = transactions.len() as u64;
    let (block_header_rlp, receipts) = final_chain.finalize_block_with_rewards_facts_at(
        pbft_block_rlp,
        transactions,
        finalized_dag_blocks,
        blocks_per_year,
        cert_votes,
        session.block_number,
    )?;
    Ok(FinalChainExecutionCommitReport {
        status: FINAL_CHAIN_EXECUTION_STATUS_COMPLETE,
        period: session.block_number,
        block_header_rlp,
        receipts,
        gas_used: FinalChainGas::ZERO,
        executed_dag_blocks,
        executed_transactions,
        error_code: String::new(),
    })
}

/// Exact bridge-contract facts requested by the native FinalChain task.
///
/// The concrete executor fills only StateAPI-owned observations. Rust retains
/// system-transaction policy and canonical transaction encoding.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainSystemTransactionFactsRequest {
    pub request_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub is_pillar_block_period: bool,
    pub bridge_contract_address: [u8; 20],
    pub block_gas_limit: FinalChainGas,
}

/// Terminal result of one application-root FinalChain execution task.
///
/// No session, action cursor, publication plan, or storage handle escapes in
/// this report. Successful reports describe an already durable native
/// FinalChain publication.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainApplicationExecutionReport {
    pub period: FinalChainBlockNumber,
    pub block_hash: [u8; 32],
    pub executed_dag_blocks: u64,
    pub executed_transactions: u64,
    pub status: u8,
    pub error_code: String,
}

/// Concrete EVM and `state_db` leaves used by native FinalChain execution.
///
/// Implementations must not plan consensus work or publish FinalChain storage.
/// Calls are ordered by [`execute_final_chain_application_task`]; failures
/// after staged EVM execution are fail-closed because the current StateAPI has
/// no safe in-process discard primitive.
pub trait FinalChainExecutionLeaf {
    /// Loads the concrete committed descriptor without opening or mutating a
    /// staged transition. The report must echo the request identity.
    fn load_committed_state_descriptor(
        &self,
        request: &FinalChainExternalEvmPreflightRequest,
    ) -> Result<FinalChainExternalEvmPreflightReport, anyhow::Error>;

    /// Loads read-only bridge-contract facts for Rust system-transaction
    /// planning. Implementations must not select or encode transactions.
    fn load_system_transaction_facts(
        &self,
        request: &FinalChainSystemTransactionFactsRequest,
    ) -> Result<FinalChainSystemTransactionPlanFact, anyhow::Error>;

    /// Executes the complete ordered transaction stream from `prior_state` and
    /// returns exact receipt facts and the post-transaction root without
    /// committing state.
    fn execute_transactions(
        &self,
        request: &FinalChainEvmExecutionRequest,
    ) -> Result<FinalChainEvmExecutionReport, anyhow::Error>;

    /// Applies the Rust-planned rewards boundary to the same staged transition
    /// and returns the exact post-rewards root without committing state.
    fn distribute_rewards(
        &self,
        request: &FinalChainEvmRewardsRequest,
    ) -> Result<FinalChainEvmRewardsReport, anyhow::Error>;

    /// Attempts the already-approved concrete commit and reports the exact
    /// descriptor observed afterward. It must not publish FinalChain storage.
    fn commit_staged_state(
        &self,
        request: &FinalChainExternalEvmStateCommitIntent,
    ) -> Result<FinalChainExternalEvmStateCommitResult, anyhow::Error>;

    /// Discards the exact staged marker and reopens concrete execution at the
    /// marker's prior committed descriptor.
    fn discard_staged_state(
        &self,
        _request: &FinalChainExternalEvmDiscardRequest,
    ) -> Result<FinalChainExternalEvmDiscardReport, anyhow::Error> {
        anyhow::bail!("FINAL_CHAIN_CONCRETE_DISCARD_LEAF_UNAVAILABLE")
    }
}

fn committed_application_report(
    final_chain: &FinalChain,
    commit: FinalChainExecutionCommitReport,
) -> Result<FinalChainApplicationExecutionReport, anyhow::Error> {
    let block_hash: [u8; 32] = final_chain
        .block_hash(commit.period)?
        .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_APPLICATION_COMMITTED_HASH_MISSING"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("FINAL_CHAIN_APPLICATION_COMMITTED_HASH_INVALID_LENGTH"))?;
    Ok(FinalChainApplicationExecutionReport {
        period: commit.period,
        block_hash,
        executed_dag_blocks: commit.executed_dag_blocks,
        executed_transactions: commit.executed_transactions,
        status: commit.status,
        error_code: commit.error_code,
    })
}

fn discard_concrete_after_failure<E: FinalChainExecutionLeaf>(
    final_chain: &FinalChain,
    leaf: &E,
    request: &FinalChainEvmExecutionRequest,
    cause: anyhow::Error,
) -> anyhow::Error {
    if let Err(marker_error) = decode_concrete_execution_marker(&request.concrete_marker_rlp) {
        return anyhow::anyhow!(
            "{cause:#}; FINAL_CHAIN_CONCRETE_DISCARD_MARKER_INVALID: {marker_error:#}"
        );
    }
    let marker_hash = concrete_state_bytes_digest(&request.concrete_marker_rlp);
    let discard_request = FinalChainExternalEvmDiscardRequest {
        request_id: request.request_id,
        period: request.period,
        concrete_marker_rlp: request.concrete_marker_rlp.clone(),
        marker_hash,
        prior_state: request.prior_state,
    };
    let discard = match leaf.discard_staged_state(&discard_request) {
        Ok(report) => report,
        Err(discard_error) => {
            return anyhow::anyhow!(
                "{cause:#}; FINAL_CHAIN_CONCRETE_DISCARD_FAILED: {discard_error:#}"
            );
        }
    };
    if !discard.succeeded
        || discard.request_id != discard_request.request_id
        || discard.period != discard_request.period
        || discard.concrete_marker_rlp != discard_request.concrete_marker_rlp
        || discard.marker_hash != marker_hash
        || discard.prior_state != request.prior_state
        || discard.committed_state != request.prior_state
    {
        return anyhow::anyhow!(
            "{cause:#}; FINAL_CHAIN_CONCRETE_DISCARD_REPORT_MISMATCH: {}",
            discard.error_code
        );
    }
    if let Err(clear_error) = final_chain.clear_external_evm_pending_publication_marker() {
        return anyhow::anyhow!(
            "{cause:#}; FINAL_CHAIN_CONCRETE_PENDING_PUBLICATION_CLEAR_AFTER_DISCARD_FAILED: {clear_error:#}"
        );
    }
    cause
}

/// Reopens StateAPI after an ambiguous concrete commit call and classifies the
/// only two safe outcomes. An exact prior descriptor is discarded/cleared; an
/// exact planned descriptor is accepted only with byte-identical Rust-authored
/// provenance and no staged marker. Every other observation leaves the durable
/// pending-publication marker intact for startup recovery.
fn classify_ambiguous_concrete_commit<E: FinalChainExecutionLeaf>(
    final_chain: &FinalChain,
    leaf: &E,
    request: &FinalChainEvmExecutionRequest,
    intent: &FinalChainExternalEvmStateCommitIntent,
    cause: anyhow::Error,
) -> Result<FinalChainExternalEvmStateCommitResult, anyhow::Error> {
    let marker = decode_concrete_execution_marker(&request.concrete_marker_rlp)
        .context("FINAL_CHAIN_CONCRETE_AMBIGUOUS_COMMIT_MARKER_INVALID")?;
    let observed = leaf
        .load_committed_state_descriptor(&FinalChainExternalEvmPreflightRequest {
            request_id: request.request_id,
            next_period: request.period,
            expected_prior: request.prior_state,
            concrete_chain_identity: marker.identity.chain_id,
        })
        .with_context(|| format!("{cause:#}; FINAL_CHAIN_CONCRETE_COMMIT_REOPEN_FAILED"))?;
    ensure!(
        observed.succeeded && observed.request_id == request.request_id,
        "{cause:#}; FINAL_CHAIN_CONCRETE_COMMIT_REOPEN_IDENTITY_MISMATCH: {}",
        observed.error_code
    );
    let observed_provenance = decode_concrete_state_provenance(&observed.concrete_provenance_rlp)
        .context("FINAL_CHAIN_CONCRETE_COMMIT_REOPEN_PROVENANCE_INVALID")?;
    final_chain.verify_or_initialize_concrete_state_pairing(observed_provenance.identity)?;

    let expected_committed = FinalChainExternalEvmCommittedStateDescriptor {
        period: intent.period,
        state_root: intent.post_rewards_state_root,
    };
    if observed.committed == expected_committed {
        ensure!(
            observed.pending_concrete_marker_rlp.is_empty(),
            "{cause:#}; FINAL_CHAIN_CONCRETE_COMMIT_REOPEN_COMMITTED_WITH_PENDING_MARKER"
        );
        ensure!(
            observed.concrete_provenance_rlp == intent.concrete_provenance_rlp,
            "{cause:#}; FINAL_CHAIN_CONCRETE_COMMIT_REOPEN_PROVENANCE_MISMATCH"
        );
        return Ok(FinalChainExternalEvmStateCommitResult {
            request_id: intent.request_id,
            plan_id: intent.plan_id,
            period: intent.period,
            publication_block_hash: intent.publication_block_hash,
            prior_state: intent.prior_state,
            post_transaction_state_root: intent.post_transaction_state_root,
            post_rewards_state_root: intent.post_rewards_state_root,
            concrete_marker_rlp: intent.concrete_marker_rlp.clone(),
            concrete_projection_rlp: intent.concrete_projection_rlp.clone(),
            concrete_projection_hash: intent.concrete_projection_hash,
            concrete_provenance_rlp: intent.concrete_provenance_rlp.clone(),
            committed_state: Some(expected_committed),
            status: FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED,
            error_code: String::new(),
        });
    }
    if observed.committed == request.prior_state {
        if observed.pending_concrete_marker_rlp == request.concrete_marker_rlp {
            return Err(discard_concrete_after_failure(
                final_chain,
                leaf,
                request,
                cause,
            ));
        }
        ensure!(
            observed.pending_concrete_marker_rlp.is_empty(),
            "{cause:#}; FINAL_CHAIN_CONCRETE_COMMIT_REOPEN_PRIOR_WITH_FOREIGN_MARKER"
        );
        final_chain
            .clear_external_evm_pending_publication_marker()
            .with_context(|| {
                format!("{cause:#}; FINAL_CHAIN_CONCRETE_PENDING_PUBLICATION_CLEAR_FAILED")
            })?;
        return Err(cause);
    }
    anyhow::bail!(
        "{cause:#}; FINAL_CHAIN_CONCRETE_COMMIT_REOPEN_AMBIGUOUS_DESCRIPTOR: period {} root {:02x?}",
        observed.committed.period.as_u64(),
        observed.committed.state_root
    )
}

/// Authorizes cleanup of a StateAPI transition staged before Rust could
/// persist its publication marker.
///
/// All crash points from immediately after physical staging through rewards
/// planning are externally equivalent: StateAPI still reports the exact prior
/// committed descriptor plus one next-generation marker, while native
/// FinalChain has no pending publication. Such state is safe only to discard;
/// a foreign identity, generation, prior descriptor, or non-consecutive period
/// is never treated as retryable work.
fn orphaned_concrete_discard_request(
    expected_prior: FinalChainExternalEvmCommittedStateDescriptor,
    provenance: &FinalChainConcreteStateProvenance,
    pending_marker_rlp: &[u8],
) -> Result<FinalChainExternalEvmDiscardRequest, anyhow::Error> {
    let marker = decode_concrete_execution_marker(pending_marker_rlp)
        .context("FINAL_CHAIN_CONCRETE_RECOVERY_ORPHAN_MARKER_INVALID")?;
    ensure!(
        marker.identity == provenance.identity,
        "FINAL_CHAIN_CONCRETE_RECOVERY_ORPHAN_IDENTITY_MISMATCH"
    );
    ensure!(
        marker.generation
            == provenance
                .generation
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!(
                    "FINAL_CHAIN_CONCRETE_RECOVERY_ORPHAN_GENERATION_OVERFLOW"
                ))?,
        "FINAL_CHAIN_CONCRETE_RECOVERY_ORPHAN_GENERATION_MISMATCH"
    );
    ensure!(
        marker.prior_state.period == expected_prior.period.as_u64()
            && marker.prior_state.root == expected_prior.state_root
            && provenance.committed_state.period == expected_prior.period.as_u64()
            && provenance.committed_state.root == expected_prior.state_root,
        "FINAL_CHAIN_CONCRETE_RECOVERY_ORPHAN_PRIOR_MISMATCH"
    );
    let expected_period = expected_prior
        .period
        .checked_next()
        .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_CONCRETE_RECOVERY_PERIOD_OVERFLOW"))?;
    ensure!(
        marker.period == expected_period.as_u64(),
        "FINAL_CHAIN_CONCRETE_RECOVERY_ORPHAN_PERIOD_MISMATCH"
    );
    Ok(FinalChainExternalEvmDiscardRequest {
        request_id: concrete_state_bytes_digest(pending_marker_rlp),
        period: expected_period,
        concrete_marker_rlp: pending_marker_rlp.to_vec(),
        marker_hash: concrete_state_bytes_digest(pending_marker_rlp),
        prior_state: expected_prior,
    })
}

/// Recovers the paired concrete-state and native FinalChain lifecycle through
/// one application-owned operation.
///
/// The concrete leaf only opens state, reports durable facts, and executes an
/// exact marker discard authorized by Rust. Rust derives the chain identity,
/// validates every observed descriptor/provenance transition, and owns the
/// retry and FinalChain publication decision.
pub fn recover_final_chain_application_state<E: FinalChainExecutionLeaf>(
    final_chain: &FinalChain,
    leaf: &E,
) -> Result<FinalChainExternalEvmPublicationReport, anyhow::Error> {
    let expected_prior = final_chain.committed_state_descriptor()?;
    let concrete_chain_identity = final_chain.concrete_chain_identity()?;
    let request_id = [0; 32];
    let load = || {
        leaf.load_committed_state_descriptor(&FinalChainExternalEvmPreflightRequest {
            request_id,
            next_period: expected_prior
                .period
                .checked_next()
                .unwrap_or(expected_prior.period),
            expected_prior,
            concrete_chain_identity,
        })
    };
    let mut observed = load()?;
    ensure!(
        observed.succeeded && observed.request_id == request_id,
        "FINAL_CHAIN_CONCRETE_RECOVERY_PREFLIGHT_FAILED: {}",
        observed.error_code
    );
    let provenance = decode_concrete_state_provenance(&observed.concrete_provenance_rlp)
        .context("FINAL_CHAIN_CONCRETE_RECOVERY_PROVENANCE_INVALID")?;
    final_chain.verify_or_initialize_concrete_state_pairing(provenance.identity)?;
    ensure!(
        provenance.identity.chain_id == concrete_chain_identity
            && provenance.committed_state.period == observed.committed.period.as_u64()
            && provenance.committed_state.root == observed.committed.state_root,
        "FINAL_CHAIN_CONCRETE_RECOVERY_PROVENANCE_MISMATCH"
    );
    let has_pending_publication = final_chain.has_external_evm_pending_publication()?;
    if !has_pending_publication {
        ensure!(
            observed.committed == expected_prior,
            "FINAL_CHAIN_CONCRETE_RECOVERY_UNPLANNED_DESCRIPTOR"
        );
        if !observed.pending_concrete_marker_rlp.is_empty() {
            let discard = orphaned_concrete_discard_request(
                expected_prior,
                &provenance,
                &observed.pending_concrete_marker_rlp,
            )?;
            let discarded = leaf.discard_staged_state(&discard)?;
            ensure!(
                discarded.succeeded
                    && discarded.request_id == discard.request_id
                    && discarded.period == discard.period
                    && discarded.concrete_marker_rlp == discard.concrete_marker_rlp
                    && discarded.marker_hash == discard.marker_hash
                    && discarded.prior_state == discard.prior_state
                    && discarded.committed_state == discard.prior_state,
                "FINAL_CHAIN_CONCRETE_RECOVERY_ORPHAN_DISCARD_MISMATCH: {}",
                discarded.error_code
            );
            let reopened = load()?;
            ensure!(
                reopened.succeeded
                    && reopened.request_id == request_id
                    && reopened.committed == expected_prior
                    && reopened.pending_concrete_marker_rlp.is_empty()
                    && reopened.concrete_provenance_rlp == observed.concrete_provenance_rlp,
                "FINAL_CHAIN_CONCRETE_RECOVERY_ORPHAN_REOPEN_MISMATCH: {}",
                reopened.error_code
            );
            observed = reopened;
        }
    } else {
        let expected_next = expected_prior
            .period
            .checked_next()
            .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_CONCRETE_RECOVERY_PERIOD_OVERFLOW"))?;
        ensure!(
            observed.committed == expected_prior || observed.committed.period == expected_next,
            "FINAL_CHAIN_CONCRETE_RECOVERY_DESCRIPTOR_GAP"
        );
    }
    let mut report = final_chain.recover_external_evm_pending_publication(
        observed.committed.period.as_u64(),
        observed.committed.state_root,
        observed.concrete_provenance_rlp,
        observed.pending_concrete_marker_rlp,
    )?;
    if !report.recovery_discard_required {
        return Ok(report);
    }
    ensure!(
        report.status == FINAL_CHAIN_EVM_PUBLICATION_STATUS_REJECTED
            && report.error_code == "FINAL_CHAIN_EVM_RECOVERY_EXACT_DISCARD_REQUIRED"
            && report.recovery_concrete_chain_identity == concrete_chain_identity,
        "FINAL_CHAIN_CONCRETE_RECOVERY_DISCARD_NOT_AUTHORIZED"
    );
    let discard = FinalChainExternalEvmDiscardRequest {
        request_id: report.recovery_request_id,
        period: report.recovery_period,
        concrete_marker_rlp: report.recovery_concrete_marker_rlp.clone(),
        marker_hash: report.recovery_marker_hash,
        prior_state: report.recovery_prior_state,
    };
    let discarded = leaf.discard_staged_state(&discard)?;
    ensure!(
        discarded.succeeded
            && discarded.request_id == discard.request_id
            && discarded.period == discard.period
            && discarded.concrete_marker_rlp == discard.concrete_marker_rlp
            && discarded.marker_hash == discard.marker_hash
            && discarded.prior_state == discard.prior_state
            && discarded.committed_state == discard.prior_state,
        "FINAL_CHAIN_CONCRETE_RECOVERY_DISCARD_MISMATCH: {}",
        discarded.error_code
    );
    let reopened = load()?;
    ensure!(
        reopened.succeeded
            && reopened.request_id == request_id
            && reopened.committed == discard.prior_state
            && reopened.pending_concrete_marker_rlp.is_empty(),
        "FINAL_CHAIN_CONCRETE_RECOVERY_REOPEN_MISMATCH: {}",
        reopened.error_code
    );
    report = final_chain.recover_external_evm_pending_publication(
        reopened.committed.period.as_u64(),
        reopened.committed.state_root,
        reopened.concrete_provenance_rlp,
        reopened.pending_concrete_marker_rlp,
    )?;
    ensure!(
        !report.recovery_discard_required
            && matches!(
                report.status,
                FINAL_CHAIN_EVM_PUBLICATION_STATUS_APPLIED
                    | FINAL_CHAIN_EVM_PUBLICATION_STATUS_ALREADY_APPLIED
            ),
        "FINAL_CHAIN_CONCRETE_RECOVERY_RETRY_REJECTED: {}",
        report.error_code
    );
    Ok(report)
}

/// Executes one complete FinalChain task behind the application root.
///
/// Rust owns session progression, system-transaction and reward planning,
/// report/receipt/root validation, durable pending-marker ordering, and atomic
/// FinalChain publication. The leaf owns only concrete EVM and state-db calls.
/// The function never exposes the private session transcript to C++.
pub fn execute_final_chain_application_task<E: FinalChainExecutionLeaf>(
    final_chain: &FinalChain,
    request: FinalChainExecutionRequest,
    proposal_period_update: FinalChainProposalPeriodDagLevelUpdate,
    is_pillar_block_period: bool,
    bridge_contract_address: [u8; 20],
    leaf: &E,
) -> Result<FinalChainApplicationExecutionReport, anyhow::Error> {
    let mut session = create_final_chain_execution_session(request);
    let expected_prior = final_chain.committed_state_descriptor()?;
    let expected_next = expected_prior
        .period
        .checked_next()
        .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_APPLICATION_PRIOR_PERIOD_OVERFLOW"))?;
    ensure!(
        session.block_number == expected_next,
        "FINAL_CHAIN_APPLICATION_NON_CONSECUTIVE_PERIOD: expected {}, requested {}",
        expected_next.as_u64(),
        session.block_number.as_u64()
    );
    let mut step = final_chain_execution_session_next(&mut session);

    if step.action == FINAL_CHAIN_EXECUTION_ACTION_COMMIT_NATIVE {
        let commit = commit_final_chain_execution_session(final_chain, session)?;
        return committed_application_report(final_chain, commit);
    }

    if step.action == FINAL_CHAIN_EXECUTION_ACTION_PROVIDE_SYSTEM_TRANSACTIONS {
        let system_request = &step.system_transaction_request;
        let mut facts =
            leaf.load_system_transaction_facts(&FinalChainSystemTransactionFactsRequest {
                request_id: system_request.request_id,
                period: system_request.period,
                is_pillar_block_period,
                bridge_contract_address,
                block_gas_limit: session.request.block_gas_limit,
            })?;
        facts.request_id = system_request.request_id;
        facts.period = system_request.period;
        facts.is_pillar_block_period = is_pillar_block_period;
        facts.bridge_contract_address = bridge_contract_address;
        facts.block_gas_limit = session.request.block_gas_limit;
        let plan = plan_external_evm_system_transactions(facts)?;
        step = final_chain_execution_session_report_system_transactions(
            &mut session,
            FinalChainSystemTransactionReport {
                request_id: plan.request_id,
                period: plan.period,
                transactions: plan.transactions,
            },
        );
    }

    if step.action == FINAL_CHAIN_EXECUTION_ACTION_COMMIT_NATIVE {
        let commit = commit_final_chain_execution_session(final_chain, session)?;
        return committed_application_report(final_chain, commit);
    }

    if step.action != FINAL_CHAIN_EXECUTION_ACTION_EXECUTE_EXTERNAL_EVM {
        anyhow::bail!(
            "FINAL_CHAIN_APPLICATION_EXPECTED_EVM_EXECUTION: {}: {}",
            step.action,
            step.error_code
        );
    }
    let mut bound_evm_request =
        final_chain_execution_session_bind_external_evm_prior_state(&mut session, expected_prior)?;
    let concrete_chain_identity = final_chain.concrete_chain_identity()?;
    let preflight =
        leaf.load_committed_state_descriptor(&FinalChainExternalEvmPreflightRequest {
            request_id: bound_evm_request.request_id,
            next_period: session.block_number,
            expected_prior,
            concrete_chain_identity,
        })?;
    ensure!(
        preflight.succeeded,
        "FINAL_CHAIN_EXTERNAL_EVM_PREFLIGHT_FAILED: {}",
        preflight.error_code
    );
    ensure!(
        preflight.request_id == bound_evm_request.request_id,
        "FINAL_CHAIN_EXTERNAL_EVM_PREFLIGHT_REQUEST_ID_MISMATCH"
    );
    ensure!(
        preflight.committed.period == expected_prior.period
            && preflight.committed.state_root == expected_prior.state_root,
        "FINAL_CHAIN_EXTERNAL_EVM_PRIOR_DESCRIPTOR_MISMATCH: expected period {} root {:02x?}, observed period {} root {:02x?}",
        expected_prior.period.as_u64(),
        expected_prior.state_root,
        preflight.committed.period.as_u64(),
        preflight.committed.state_root
    );
    let provenance = decode_concrete_state_provenance(&preflight.concrete_provenance_rlp)
        .context("FINAL_CHAIN_CONCRETE_PREFLIGHT_PROVENANCE_INVALID")?;
    final_chain.verify_or_initialize_concrete_state_pairing(provenance.identity)?;
    ensure!(
        provenance.committed_state.period == expected_prior.period.as_u64()
            && provenance.committed_state.root == expected_prior.state_root,
        "FINAL_CHAIN_CONCRETE_PREFLIGHT_PROVENANCE_DESCRIPTOR_MISMATCH"
    );
    ensure!(
        preflight.pending_concrete_marker_rlp.is_empty(),
        "FINAL_CHAIN_CONCRETE_PENDING_EXECUTION_REQUIRES_RECOVERY"
    );
    let transactions_hash = concrete_transactions_hash(&bound_evm_request.transactions);
    let rewards_hash = concrete_rewards_plan_hash(&session.request, &bound_evm_request);
    let concrete_plan_hash = concrete_execution_plan_hash(
        bound_evm_request.request_id,
        bound_evm_request.period,
        bound_evm_request.prior_state,
        transactions_hash,
        rewards_hash,
    );
    let marker = FinalChainConcreteExecutionMarker {
        identity: provenance.identity,
        generation: provenance
            .generation
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_CONCRETE_GENERATION_OVERFLOW"))?,
        plan_hash: concrete_plan_hash,
        period: bound_evm_request.period.as_u64(),
        prior_state: FinalChainConcreteState {
            period: expected_prior.period.as_u64(),
            root: expected_prior.state_root,
        },
        transactions_hash,
        rewards_hash,
    };
    bound_evm_request.concrete_marker_rlp = encode_concrete_execution_marker(&marker);
    bound_evm_request.concrete_plan_hash = concrete_plan_hash;
    bound_evm_request.transactions_hash = transactions_hash;
    bound_evm_request.rewards_hash = rewards_hash;
    session.evm_request = Some(bound_evm_request.clone());
    let execution_report = match leaf.execute_transactions(&bound_evm_request) {
        Ok(report) => report,
        Err(error) => {
            return Err(discard_concrete_after_failure(
                final_chain,
                leaf,
                &bound_evm_request,
                error,
            ));
        }
    };
    step = final_chain_execution_session_report_evm_with_final_chain(
        final_chain,
        &mut session,
        execution_report,
    );
    if step.action != FINAL_CHAIN_EXECUTION_ACTION_DISTRIBUTE_EXTERNAL_EVM_REWARDS {
        let error = anyhow::anyhow!(
            "FINAL_CHAIN_APPLICATION_EXPECTED_REWARDS: {}: {}",
            step.action,
            step.error_code
        );
        return Err(discard_concrete_after_failure(
            final_chain,
            leaf,
            &bound_evm_request,
            error,
        ));
    }
    let rewards_report = match leaf.distribute_rewards(&step.evm_rewards_request) {
        Ok(report) => report,
        Err(error) => {
            return Err(discard_concrete_after_failure(
                final_chain,
                leaf,
                &bound_evm_request,
                error,
            ));
        }
    };
    let commit_plan =
        final_chain_execution_session_plan_external_evm_commit(&mut session, rewards_report);
    if !commit_plan.error_code.is_empty() {
        return Err(discard_concrete_after_failure(
            final_chain,
            leaf,
            &bound_evm_request,
            anyhow::anyhow!(
                "FINAL_CHAIN_APPLICATION_COMMIT_PLAN_REJECTED: {}",
                commit_plan.error_code
            ),
        ));
    }
    let intent = match final_chain_execution_session_prepare_external_evm_state_commit(
        final_chain,
        &mut session,
        proposal_period_update,
    ) {
        Ok(intent) => intent,
        Err(error) => {
            return Err(discard_concrete_after_failure(
                final_chain,
                leaf,
                &bound_evm_request,
                error,
            ));
        }
    };
    if intent.status != FINAL_CHAIN_EVM_STATE_COMMIT_INTENT_READY_TO_COMMIT
        || !intent.error_code.is_empty()
    {
        return Err(discard_concrete_after_failure(
            final_chain,
            leaf,
            &bound_evm_request,
            anyhow::anyhow!(
                "FINAL_CHAIN_APPLICATION_STATE_COMMIT_NOT_READY: {}",
                intent.error_code
            ),
        ));
    }

    // The pending-publication marker is persisted by the preparation call
    // before this concrete state-db commit is attempted.
    let mut state_commit = match leaf.commit_staged_state(&intent) {
        Ok(result) => result,
        Err(error) => classify_ambiguous_concrete_commit(
            final_chain,
            leaf,
            &bound_evm_request,
            &intent,
            error,
        )?,
    };
    let commit_plan = session
        .external_evm_commit_plan
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_APPLICATION_COMMIT_PLAN_MISSING"))?;
    if state_commit.status != FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED
        || !state_commit.error_code.is_empty()
        || validate_external_evm_state_commit_result_facts(&intent, commit_plan, &state_commit)
            .is_err()
    {
        state_commit = classify_ambiguous_concrete_commit(
            final_chain,
            leaf,
            &bound_evm_request,
            &intent,
            anyhow::anyhow!(
                "FINAL_CHAIN_APPLICATION_STATE_COMMIT_REPORT_AMBIGUOUS: {}",
                state_commit.error_code
            ),
        )?;
    }
    let decision = final_chain_execution_session_report_external_evm_state_commit_result(
        final_chain,
        &mut session,
        state_commit,
    )?;
    if decision.status != FINAL_CHAIN_EVM_COMMIT_DECISION_READY_TO_PUBLISH
        || !decision.error_code.is_empty()
    {
        let error = anyhow::anyhow!(
            "FINAL_CHAIN_APPLICATION_STATE_COMMIT_REJECTED: {}",
            decision.error_code
        );
        return Err(error);
    }
    let publication =
        final_chain_execution_session_publish_external_evm_publication(final_chain, &mut session)?;
    ensure!(
        matches!(
            publication.status,
            FINAL_CHAIN_EVM_PUBLICATION_STATUS_APPLIED
                | FINAL_CHAIN_EVM_PUBLICATION_STATUS_ALREADY_APPLIED
        ),
        "FINAL_CHAIN_APPLICATION_PUBLICATION_REJECTED: {}",
        publication.error_code
    );
    Ok(FinalChainApplicationExecutionReport {
        period: publication.period,
        block_hash: publication.block_hash,
        executed_dag_blocks: publication.executed_dag_block_count,
        executed_transactions: publication.executed_transaction_count,
        status: publication.status,
        error_code: publication.error_code,
    })
}

impl FinalChainExecutionSession {
    fn new(
        request: FinalChainExecutionRequest,
        metadata: rustaxa_types::PbftBlockMetadata,
    ) -> Self {
        let regular_transaction_count = request.transactions.len();
        let block_number = FinalChainBlockNumber::new(metadata.period);
        Self::new_with_regular_transaction_count(
            request,
            metadata,
            block_number,
            regular_transaction_count,
        )
    }

    fn new_with_regular_transaction_count(
        request: FinalChainExecutionRequest,
        metadata: rustaxa_types::PbftBlockMetadata,
        block_number: FinalChainBlockNumber,
        regular_transaction_count: usize,
    ) -> Self {
        if let Err(error_code) = validate_regular_transaction_count(regular_transaction_count) {
            return Self::rejected(request, metadata, block_number, error_code.to_string());
        }
        let ordered_transactions = classify_ordered_execution_transactions(&request.transactions)
            .expect("regular transaction count was validated");
        let external_evm_transaction_count = count_external_evm_transactions(&ordered_transactions);
        if external_evm_transaction_count == 0
            && request.mode != FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED
        {
            return Self {
                request,
                metadata,
                block_number,
                evm_request: None,
                status: FINAL_CHAIN_EXECUTION_STATUS_READY,
                system_transaction_request: None,
                system_transactions: Vec::new(),
                report: None,
                rewards_request: None,
                prepared_rewards_stats_plan: None,
                external_evm_commit_plan: None,
                external_evm_publication_plan: None,
                external_evm_state_commit_intent: None,
                external_evm_commit_decision: None,
                error_code: String::new(),
            };
        }
        if request.mode != FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED {
            return Self::rejected(
                request,
                metadata,
                block_number,
                "FINAL_CHAIN_EXECUTION_REQUIRES_EXTERNAL_EVM".to_string(),
            );
        }
        let system_transaction_request = FinalChainSystemTransactionRequest {
            request_id: system_transaction_request_id(
                block_number,
                &metadata,
                request.block_gas_limit,
                &ordered_transactions,
            ),
            period: block_number,
            regular_transaction_count: ordered_transactions.len() as u64,
        };
        Self {
            request,
            metadata,
            block_number,
            evm_request: None,
            status: FINAL_CHAIN_EXECUTION_STATUS_READY,
            system_transaction_request: Some(system_transaction_request),
            system_transactions: Vec::new(),
            report: None,
            rewards_request: None,
            prepared_rewards_stats_plan: None,
            external_evm_commit_plan: None,
            external_evm_publication_plan: None,
            external_evm_state_commit_intent: None,
            external_evm_commit_decision: None,
            error_code: String::new(),
        }
    }

    fn rejected(
        request: FinalChainExecutionRequest,
        metadata: rustaxa_types::PbftBlockMetadata,
        block_number: FinalChainBlockNumber,
        error_code: String,
    ) -> Self {
        Self {
            request,
            metadata,
            block_number,
            evm_request: None,
            status: FINAL_CHAIN_EXECUTION_STATUS_REJECTED,
            system_transaction_request: None,
            system_transactions: Vec::new(),
            report: None,
            rewards_request: None,
            prepared_rewards_stats_plan: None,
            external_evm_commit_plan: None,
            external_evm_publication_plan: None,
            external_evm_state_commit_intent: None,
            external_evm_commit_decision: None,
            error_code,
        }
    }
}

fn rejected_session_external_evm_publication_report(
    session: &FinalChainExecutionSession,
    error_code: impl Into<String>,
) -> FinalChainExternalEvmPublicationReport {
    let (request_id, plan_id, block_hash) = session
        .external_evm_publication_plan
        .as_ref()
        .map(|plan| (plan.request_id, plan.plan_id, plan.block_hash))
        .unwrap_or_default();
    FinalChainExternalEvmPublicationReport {
        request_id,
        plan_id,
        period: session.block_number,
        block_hash,
        dpos_snapshot_status: FINAL_CHAIN_EVM_PUBLICATION_SNAPSHOT_STATUS_NOT_EVALUATED,
        account_snapshot_status: FINAL_CHAIN_EVM_PUBLICATION_SNAPSHOT_STATUS_NOT_EVALUATED,
        status: FINAL_CHAIN_EVM_PUBLICATION_STATUS_REJECTED,
        error_code: error_code.into(),
        ..Default::default()
    }
}

fn decode_system_transaction_inputs(
    regular_transaction_count: usize,
    system_transaction_rlps: &[Vec<u8>],
) -> Result<Vec<FinalChainEvmTransactionInput>, anyhow::Error> {
    system_transaction_rlps
        .iter()
        .enumerate()
        .map(|(index, rlp)| {
            let envelope = LegacyTransactionEnvelope::decode_system(rlp)
                .context("decode external EVM system transaction")?;
            let sender = envelope
                .sender
                .ok_or_else(|| anyhow::anyhow!("system transaction sender missing"))?;
            let position = regular_transaction_count
                .checked_add(index)
                .ok_or_else(|| {
                    anyhow::anyhow!("FINAL_CHAIN_SYSTEM_TRANSACTION_POSITION_EXCEEDS_U32")
                })?;
            Ok(FinalChainEvmTransactionInput {
                position: FinalChainTransactionPosition::try_from(position).map_err(|_| {
                    anyhow::anyhow!("FINAL_CHAIN_SYSTEM_TRANSACTION_POSITION_EXCEEDS_U32")
                })?,
                hash: envelope.hash.into(),
                sender: sender.into(),
                receiver: envelope.receiver.map(Into::into),
                nonce: FinalChainNonce::from_bytes(&u256_to_nonce_bytes(envelope.nonce))?,
                value: envelope.value.into(),
                gas_price: envelope.gas_price.into(),
                gas_limit: envelope.gas.into(),
                data: envelope.data,
                rlp: envelope.rlp,
                kind: FINAL_CHAIN_EXECUTION_TX_KIND_SYSTEM,
                is_system: true,
            })
        })
        .collect()
}

fn solidity_no_arg_call(signature: &str) -> Vec<u8> {
    let mut output = [0u8; 32];
    let mut hasher = tiny_keccak::Keccak::v256();
    tiny_keccak::Hasher::update(&mut hasher, signature.as_bytes());
    tiny_keccak::Hasher::finalize(hasher, &mut output);
    output[..4].to_vec()
}

fn build_external_evm_rewards_request(
    finalization_request: &FinalChainExecutionRequest,
    request: &FinalChainEvmExecutionRequest,
    report: &FinalChainEvmExecutionReport,
) -> Result<FinalChainEvmRewardsRequest, anyhow::Error> {
    let mut transaction_gas_used = Vec::with_capacity(report.results.len());
    let mut transaction_fees = Vec::with_capacity(report.results.len());
    for (transaction, result) in request.transactions.iter().zip(report.results.iter()) {
        let fee = transaction
            .gas_price
            .checked_fee(result.gas_used)
            .ok_or_else(|| anyhow::anyhow!("external EVM transaction fee overflow"))?;
        transaction_gas_used.push(result.gas_used);
        transaction_fees.push(u256_to_big_endian(fee));
    }
    Ok(FinalChainEvmRewardsRequest {
        request_id: request.request_id,
        period: request.period,
        prior_state: request.prior_state,
        post_transaction_state_root: report.post_transaction_state_root,
        concrete_marker_rlp: request.concrete_marker_rlp.clone(),
        concrete_plan_hash: request.concrete_plan_hash,
        transactions_hash: request.transactions_hash,
        rewards_hash: request.rewards_hash,
        block_author: request.block_author,
        block_gas_used: report.cumulative_gas_used,
        transaction_gas_used,
        transaction_fees,
        finalized_dag_block_count: finalization_request.finalized_dag_blocks.len() as u64,
        distribution_stats: Vec::new(),
    })
}

fn build_external_evm_rewards_fact(
    finalization_request: &FinalChainExecutionRequest,
    request: &FinalChainEvmExecutionRequest,
    report: &FinalChainEvmExecutionReport,
) -> Result<FinalizedRewardsPeriodFact, anyhow::Error> {
    let dpos_eligible_total_vote_count = finalization_request
        .cert_votes
        .first()
        .map(|vote| vote.period.saturating_sub(1));
    Ok(FinalizedRewardsPeriodFact {
        period: request.period.as_u64(),
        block_author: H160::from(request.block_author),
        blocks_per_year: finalization_request.blocks_per_year,
        // FinalChain replaces this placeholder from its period-keyed DPoS
        // snapshot before invoking the storage-free planner.
        dpos_eligible_total_vote_count: dpos_eligible_total_vote_count.unwrap_or_default(),
        transactions: request
            .transactions
            .iter()
            .zip(&report.results)
            .map(|(transaction, result)| RewardTransactionFact {
                hash: H256::from(transaction.hash),
                gas_price: transaction.gas_price.as_u256(),
                gas_used: result.gas_used,
            })
            .collect(),
        dag_blocks: finalization_request
            .finalized_dag_blocks
            .iter()
            .map(|block| RewardDagBlockFact {
                author: H160::from(block.author),
                difficulty: block.difficulty,
                transaction_hashes: block
                    .transaction_hashes
                    .iter()
                    .copied()
                    .map(H256::from)
                    .collect(),
            })
            .collect(),
        cert_votes: finalization_request.cert_votes.clone(),
    })
}

fn build_external_evm_commit_plan(
    request: &FinalChainExecutionRequest,
    metadata: &rustaxa_types::PbftBlockMetadata,
    block_number: FinalChainBlockNumber,
    evm_request: &FinalChainEvmExecutionRequest,
    evm_report: &FinalChainEvmExecutionReport,
    rewards_report: &FinalChainEvmRewardsReport,
) -> Result<FinalChainExternalEvmCommitPlan, anyhow::Error> {
    if evm_request.transactions.len() < request.transactions.len() {
        anyhow::bail!(
            "external EVM request has {} transaction(s), fewer than finalization request {}",
            evm_request.transactions.len(),
            request.transactions.len()
        );
    }
    let encoded_receipts = evm_report
        .results
        .iter()
        .map(validate_and_clone_external_evm_receipt)
        .collect::<Result<Vec<_>, _>>()?;
    let receipts_rlp = encode_receipts_rlp(&encoded_receipts);
    let header_log_bloom =
        block_log_bloom(evm_report.results.iter().flat_map(|result| &result.logs));
    let mut indexed_log_bloom = header_log_bloom;
    add_bloom_value(&mut indexed_log_bloom, metadata.author.as_bytes());
    Ok(FinalChainExternalEvmCommitPlan {
        request_id: evm_request.request_id,
        period: block_number,
        prior_state: evm_request.prior_state,
        post_transaction_state_root: evm_report.post_transaction_state_root,
        post_rewards_state_root: rewards_report.post_rewards_state_root,
        concrete_marker_rlp: rewards_report.concrete_marker_rlp.clone(),
        concrete_plan_hash: rewards_report.concrete_plan_hash,
        transactions_hash: rewards_report.transactions_hash,
        rewards_hash: rewards_report.rewards_hash,
        concrete_projection_rlp: rewards_report.concrete_projection_rlp.clone(),
        concrete_projection_hash: rewards_report.concrete_projection_hash,
        concrete_provenance_rlp: rewards_report.concrete_provenance_rlp.clone(),
        total_reward: rewards_report.total_reward.clone(),
        transactions_root: ordered_root(
            request
                .transactions
                .iter()
                .map(|transaction| transaction.rlp.as_slice())
                .chain(
                    evm_request.transactions[request.transactions.len()..]
                        .iter()
                        .map(|transaction| transaction.rlp.as_slice()),
                ),
        )
        .into(),
        receipts_root: ordered_root(encoded_receipts.iter().map(|receipt| receipt.as_slice()))
            .into(),
        header_log_bloom,
        indexed_log_bloom,
        receipts_rlp,
        encoded_receipts,
        gas_used: evm_report.cumulative_gas_used,
        executed_dag_blocks: request.finalized_dag_blocks.len() as u64,
        executed_transactions: evm_request.transactions.len() as u64,
        regular_transaction_count: request.transactions.len() as u64,
        system_transaction_count: evm_request.transactions.len() as u64
            - request.transactions.len() as u64,
        error_code: String::new(),
    })
}

fn build_external_evm_publication_plan(
    final_chain: &FinalChain,
    pbft_block_rlp: &[u8],
    _metadata: &rustaxa_types::PbftBlockMetadata,
    block_number: FinalChainBlockNumber,
    evm_request: &FinalChainEvmExecutionRequest,
    commit_plan: &FinalChainExternalEvmCommitPlan,
) -> Result<FinalChainExternalEvmPublicationPlan, anyhow::Error> {
    if commit_plan.error_code.is_empty() && commit_plan.request_id != evm_request.request_id {
        anyhow::bail!("external EVM publication request id mismatch");
    }
    if commit_plan.period != block_number {
        anyhow::bail!("external EVM publication period mismatch");
    }
    if commit_plan.encoded_receipts.len() != evm_request.transactions.len() {
        anyhow::bail!(
            "external EVM publication has {} receipts for {} transaction(s)",
            commit_plan.encoded_receipts.len(),
            evm_request.transactions.len()
        );
    }

    let parent_hash = final_chain
        .block_hash(final_chain.last_block_number_typed()?)?
        .map(|bytes| h256_from_slice(&bytes, "external EVM parent final-chain hash"))
        .transpose()?
        .unwrap_or_default();
    let stored_header = StoredFinalChainBlockHeader {
        parent_hash,
        state_root: H256::from(commit_plan.post_rewards_state_root),
        transactions_root: H256::from(commit_plan.transactions_root),
        receipts_root: H256::from(commit_plan.receipts_root),
        log_bloom: commit_plan.header_log_bloom,
        gas_used: commit_plan.gas_used,
        total_reward: rustaxa_types::DposTokenAmount::from(u256_from_big_endian(
            &commit_plan.total_reward,
        )),
    };
    let stored_header_rlp = StoredBlockHeaderRlpOwned::from(&stored_header);
    let full_header = LegacyBlockHeaderRlp::try_from(
        LegacyBlockHeaderRlpInput::new(
            StoredBlockHeaderRlp::new(stored_header_rlp.as_bytes()),
            final_chain.block_gas_limit().as_u64(),
            final_chain.genesis_timestamp(),
        )
        .block_number(block_number)
        .signed_pbft_block(SignedPbftBlockRlp::new(pbft_block_rlp)),
    )?;
    let block_hash = full_header.hash()?;
    let transaction_publications = evm_request
        .transactions
        .iter()
        .zip(commit_plan.encoded_receipts.iter())
        .map(|(transaction, receipt)| {
            Ok(FinalChainExternalEvmTransactionPublication {
                transaction_hash: transaction.hash,
                position: transaction.position,
                is_system: transaction.is_system,
                transaction_rlp: transaction.rlp.clone(),
                receipt_rlp: receipt.clone(),
            })
        })
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    let mut publication = FinalChainExternalEvmPublicationPlan {
        request_id: commit_plan.request_id,
        plan_id: [0; 32],
        period: block_number,
        block_hash: block_hash.into(),
        block_header_rlp: full_header.into_vec(),
        stored_header_rlp: stored_header_rlp.into_vec(),
        receipts_rlp: commit_plan.receipts_rlp.clone(),
        indexed_log_bloom: commit_plan.indexed_log_bloom,
        system_transaction_hashes_rlp: encode_system_transaction_hashes_rlp(
            evm_request
                .transactions
                .iter()
                .filter(|transaction| transaction.is_system)
                .map(|transaction| transaction.hash),
        ),
        transaction_publications,
        executed_dag_blocks: commit_plan.executed_dag_blocks,
        executed_transactions: commit_plan.executed_transactions,
        proposal_period_dag_level_update: FinalChainProposalPeriodDagLevelUpdate::default(),
        rewards_stats_update: FinalChainExternalEvmRewardsStatsUpdate::default(),
        dpos_snapshot_rlp: Vec::new(),
        account_snapshot_rlp: Vec::new(),
        concrete_marker_rlp: commit_plan.concrete_marker_rlp.clone(),
        concrete_projection_rlp: commit_plan.concrete_projection_rlp.clone(),
        concrete_projection_hash: commit_plan.concrete_projection_hash,
        concrete_provenance_rlp: commit_plan.concrete_provenance_rlp.clone(),
        error_code: String::new(),
    };
    publication.plan_id = final_chain_external_evm_publication_plan_id(&publication);
    Ok(publication)
}

fn rejected_external_evm_commit_plan(
    block_number: FinalChainBlockNumber,
    _metadata: &rustaxa_types::PbftBlockMetadata,
    error_code: String,
) -> FinalChainExternalEvmCommitPlan {
    FinalChainExternalEvmCommitPlan {
        period: block_number,
        error_code,
        ..Default::default()
    }
}

fn rejected_external_evm_publication_plan(
    block_number: FinalChainBlockNumber,
    _metadata: &rustaxa_types::PbftBlockMetadata,
    error_code: String,
) -> FinalChainExternalEvmPublicationPlan {
    FinalChainExternalEvmPublicationPlan {
        period: block_number,
        error_code,
        ..Default::default()
    }
}

fn rejected_external_evm_state_commit_intent(
    block_number: FinalChainBlockNumber,
    _metadata: &rustaxa_types::PbftBlockMetadata,
    error_code: String,
) -> FinalChainExternalEvmStateCommitIntent {
    FinalChainExternalEvmStateCommitIntent {
        period: block_number,
        status: FINAL_CHAIN_EVM_STATE_COMMIT_INTENT_REJECTED,
        error_code,
        ..Default::default()
    }
}

fn rejected_external_evm_commit_decision(
    block_number: FinalChainBlockNumber,
    _metadata: &rustaxa_types::PbftBlockMetadata,
    error_code: String,
) -> FinalChainExternalEvmCommitDecision {
    FinalChainExternalEvmCommitDecision {
        period: block_number,
        status: FINAL_CHAIN_EVM_COMMIT_DECISION_REJECTED,
        error_code,
        ..Default::default()
    }
}

fn validate_external_evm_state_commit_facts(
    session: &FinalChainExecutionSession,
    request: &FinalChainExternalEvmStateCommitRequest,
    error_prefix: &str,
) -> Result<(), String> {
    let commit_plan = session
        .external_evm_commit_plan
        .as_ref()
        .ok_or_else(|| format!("{error_prefix}_WITHOUT_COMMIT_PLAN"))?;
    let publication_plan = session
        .external_evm_publication_plan
        .as_ref()
        .ok_or_else(|| format!("{error_prefix}_WITHOUT_PUBLICATION_PLAN"))?;
    if request.request_id != commit_plan.request_id {
        return Err(format!("{error_prefix}_REQUEST_ID_MISMATCH"));
    }
    if request.plan_id != publication_plan.plan_id {
        return Err(format!("{error_prefix}_PLAN_ID_MISMATCH"));
    }
    if request.period != session.block_number {
        return Err(format!("{error_prefix}_PERIOD_MISMATCH"));
    }
    if request.prior_state != commit_plan.prior_state {
        return Err(format!("{error_prefix}_PRIOR_STATE_MISMATCH"));
    }
    if request.post_transaction_state_root != commit_plan.post_transaction_state_root {
        return Err(format!("{error_prefix}_POST_TRANSACTION_ROOT_MISMATCH"));
    }
    if request.post_rewards_state_root != commit_plan.post_rewards_state_root {
        return Err(format!("{error_prefix}_POST_REWARDS_ROOT_MISMATCH"));
    }
    if request.publication_block_hash != publication_plan.block_hash {
        return Err(format!("{error_prefix}_BLOCK_HASH_MISMATCH"));
    }
    if request.concrete_marker_rlp != commit_plan.concrete_marker_rlp
        || request.concrete_marker_rlp != publication_plan.concrete_marker_rlp
    {
        return Err(format!("{error_prefix}_CONCRETE_MARKER_MISMATCH"));
    }
    if request.concrete_projection_rlp != commit_plan.concrete_projection_rlp
        || request.concrete_projection_rlp != publication_plan.concrete_projection_rlp
        || request.concrete_projection_hash != commit_plan.concrete_projection_hash
        || request.concrete_projection_hash != publication_plan.concrete_projection_hash
        || request.concrete_projection_hash
            != concrete_state_bytes_digest(&request.concrete_projection_rlp)
    {
        return Err(format!("{error_prefix}_CONCRETE_PROJECTION_MISMATCH"));
    }
    if request.concrete_provenance_rlp != commit_plan.concrete_provenance_rlp
        || request.concrete_provenance_rlp != publication_plan.concrete_provenance_rlp
    {
        return Err(format!("{error_prefix}_CONCRETE_PROVENANCE_MISMATCH"));
    }
    let marker = decode_concrete_execution_marker(&request.concrete_marker_rlp)
        .map_err(|_| format!("{error_prefix}_CONCRETE_MARKER_INVALID"))?;
    let provenance = decode_concrete_state_provenance(&request.concrete_provenance_rlp)
        .map_err(|_| format!("{error_prefix}_CONCRETE_PROVENANCE_INVALID"))?;
    if provenance.identity != marker.identity
        || provenance.generation != marker.generation
        || provenance.plan_hash != marker.plan_hash
        || provenance.committed_state.period != request.period.as_u64()
        || provenance.committed_state.root != request.post_rewards_state_root
        || provenance.projection_hash != request.concrete_projection_hash
    {
        return Err(format!("{error_prefix}_CONCRETE_LINEAGE_MISMATCH"));
    }
    Ok(())
}

fn validate_external_evm_state_commit_result_facts(
    intent: &FinalChainExternalEvmStateCommitIntent,
    commit_plan: &FinalChainExternalEvmCommitPlan,
    result: &FinalChainExternalEvmStateCommitResult,
) -> Result<(), &'static str> {
    if result.request_id != intent.request_id {
        return Err("FINAL_CHAIN_EVM_STATE_COMMIT_RESULT_REQUEST_ID_MISMATCH");
    }
    if result.plan_id != intent.plan_id {
        return Err("FINAL_CHAIN_EVM_STATE_COMMIT_RESULT_PLAN_ID_MISMATCH");
    }
    if result.period != intent.period {
        return Err("FINAL_CHAIN_EVM_STATE_COMMIT_RESULT_PERIOD_MISMATCH");
    }
    if result.publication_block_hash != intent.publication_block_hash {
        return Err("FINAL_CHAIN_EVM_STATE_COMMIT_RESULT_BLOCK_HASH_MISMATCH");
    }
    if result.prior_state != commit_plan.prior_state {
        return Err("FINAL_CHAIN_EVM_STATE_COMMIT_RESULT_PRIOR_STATE_MISMATCH");
    }
    if result.post_transaction_state_root != commit_plan.post_transaction_state_root {
        return Err("FINAL_CHAIN_EVM_STATE_COMMIT_RESULT_POST_TRANSACTION_ROOT_MISMATCH");
    }
    if result.post_rewards_state_root != commit_plan.post_rewards_state_root {
        return Err("FINAL_CHAIN_EVM_STATE_COMMIT_RESULT_POST_REWARDS_ROOT_MISMATCH");
    }
    if result.concrete_marker_rlp != intent.concrete_marker_rlp
        || result.concrete_projection_rlp != intent.concrete_projection_rlp
        || result.concrete_projection_hash != intent.concrete_projection_hash
        || result.concrete_provenance_rlp != intent.concrete_provenance_rlp
    {
        return Err("FINAL_CHAIN_EVM_STATE_COMMIT_RESULT_CONCRETE_IDENTITY_MISMATCH");
    }
    let Ok(provenance) = decode_concrete_state_provenance(&result.concrete_provenance_rlp) else {
        return Err("FINAL_CHAIN_EVM_STATE_COMMIT_RESULT_PROVENANCE_INVALID");
    };
    if provenance.projection_hash != result.concrete_projection_hash
        || provenance.committed_state.period != result.period.as_u64()
        || provenance.committed_state.root != result.post_rewards_state_root
    {
        return Err("FINAL_CHAIN_EVM_STATE_COMMIT_RESULT_PROVENANCE_MISMATCH");
    }
    Ok(())
}

fn validate_and_clone_external_evm_receipt(
    result: &FinalChainEvmTransactionResult,
) -> Result<Vec<u8>, anyhow::Error> {
    let encoded = encode_external_evm_receipt(result);
    if encoded != result.receipt_rlp {
        anyhow::bail!("external EVM typed receipt fields do not match receipt RLP");
    }
    Ok(result.receipt_rlp.clone())
}

/// Binds every output-only host execution result to StateAPI's canonical
/// six-field `vm.ExecutionResult` projection before receipt/header planning.
///
/// StateAPI is authoritative for concrete execution while the host report is
/// the source used to build receipts and the block header. This comparison
/// therefore covers every transaction kind, including arbitrary EVM calls and
/// system transactions that Rust does not execute natively. Malformed RLP,
/// missing effects, and any semantic disagreement fail closed.
fn validate_concrete_execution_results(
    concrete_projection_rlp: &[u8],
    reported_results: &[FinalChainEvmTransactionResult],
) -> Result<(), anyhow::Error> {
    let projection = decode_concrete_state_projection(concrete_projection_rlp)
        .context("FINAL_CHAIN_CONCRETE_RESULT_PROJECTION_INVALID")?;
    ensure!(
        projection.transaction_effects.len() == reported_results.len(),
        "FINAL_CHAIN_CONCRETE_RESULT_COUNT_MISMATCH"
    );
    for (effect, reported) in projection.transaction_effects.iter().zip(reported_results) {
        let result = rlp::Rlp::new(&effect.execution_result_rlp);
        ensure!(
            result.is_list() && result.item_count()? == 6,
            "FINAL_CHAIN_CONCRETE_EXECUTION_RESULT_FIELD_COUNT_MISMATCH"
        );
        let output: Vec<u8> = result.val_at(0)?;
        let new_contract_bytes = result.at(1)?.data()?;
        ensure!(
            new_contract_bytes.len() == 20,
            "FINAL_CHAIN_CONCRETE_EXECUTION_RESULT_CONTRACT_ADDRESS_SIZE_MISMATCH"
        );
        let mut new_contract_address = [0u8; 20];
        new_contract_address.copy_from_slice(new_contract_bytes);
        let new_contract_address =
            (new_contract_address != [0; 20]).then_some(new_contract_address);
        let mut logs = Vec::new();
        for encoded_log in result.at(2)?.iter() {
            ensure!(
                encoded_log.item_count()? == 3,
                "FINAL_CHAIN_CONCRETE_EXECUTION_RESULT_LOG_FIELD_COUNT_MISMATCH"
            );
            let address_bytes = encoded_log.at(0)?.data()?;
            ensure!(
                address_bytes.len() == 20,
                "FINAL_CHAIN_CONCRETE_EXECUTION_RESULT_LOG_ADDRESS_SIZE_MISMATCH"
            );
            let mut address = [0u8; 20];
            address.copy_from_slice(address_bytes);
            let mut topics = Vec::new();
            for encoded_topic in encoded_log.at(1)?.iter() {
                let topic_bytes = encoded_topic.data()?;
                ensure!(
                    topic_bytes.len() == 32,
                    "FINAL_CHAIN_CONCRETE_EXECUTION_RESULT_LOG_TOPIC_SIZE_MISMATCH"
                );
                let mut topic = [0u8; 32];
                topic.copy_from_slice(topic_bytes);
                topics.push(FinalChainEvmLogTopic { topic });
            }
            logs.push(FinalChainEvmLog {
                address,
                topics,
                data: encoded_log.val_at(2)?,
            });
        }
        let gas_used: u64 = result.val_at(3)?;
        let code_error: String = result.val_at(4)?;
        let consensus_error: String = result.val_at(5)?;
        let status = u8::from(code_error.is_empty() && consensus_error.is_empty());
        let mut canonical = rlp::RlpStream::new_list(6);
        canonical.append(&output);
        canonical.append(&new_contract_bytes);
        canonical.begin_list(logs.len());
        for log in &logs {
            canonical.begin_list(3);
            canonical.append(&log.address.as_slice());
            canonical.begin_list(log.topics.len());
            for topic in &log.topics {
                canonical.append(&topic.topic.as_slice());
            }
            canonical.append(&log.data);
        }
        canonical.append(&gas_used);
        canonical.append(&code_error);
        canonical.append(&consensus_error);
        ensure!(
            canonical.out().as_ref() == effect.execution_result_rlp,
            "FINAL_CHAIN_CONCRETE_EXECUTION_RESULT_NON_CANONICAL"
        );

        ensure!(
            status == reported.status,
            "FINAL_CHAIN_CONCRETE_EXECUTION_RESULT_STATUS_MISMATCH"
        );
        ensure!(
            gas_used == reported.gas_used.as_u64(),
            "FINAL_CHAIN_CONCRETE_EXECUTION_RESULT_GAS_MISMATCH"
        );
        ensure!(
            logs == reported.logs,
            "FINAL_CHAIN_CONCRETE_EXECUTION_RESULT_LOGS_MISMATCH"
        );
        ensure!(
            new_contract_address == reported.new_contract_address,
            "FINAL_CHAIN_CONCRETE_EXECUTION_RESULT_CONTRACT_ADDRESS_MISMATCH"
        );
        ensure!(
            output == reported.output,
            "FINAL_CHAIN_CONCRETE_EXECUTION_RESULT_OUTPUT_MISMATCH"
        );
        ensure!(
            code_error == reported.code_error,
            "FINAL_CHAIN_CONCRETE_EXECUTION_RESULT_CODE_ERROR_MISMATCH"
        );
        ensure!(
            consensus_error == reported.consensus_error,
            "FINAL_CHAIN_CONCRETE_EXECUTION_RESULT_CONSENSUS_ERROR_MISMATCH"
        );
    }
    Ok(())
}

fn encode_external_evm_receipt(result: &FinalChainEvmTransactionResult) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new_list(5);
    stream.append(&result.status);
    stream.append(&result.gas_used.as_u64());
    stream.append(&result.cumulative_gas_used.as_u64());
    stream.begin_list(result.logs.len());
    for log in &result.logs {
        stream.begin_list(3);
        stream.append(&log.address.as_slice());
        stream.begin_list(log.topics.len());
        for topic in &log.topics {
            stream.append(&topic.topic.as_slice());
        }
        stream.append(&log.data.as_slice());
    }
    if let Some(address) = result.new_contract_address {
        stream.append(&address.as_slice());
    } else {
        stream.append(&0u8);
    }
    stream.out().to_vec()
}

fn encode_receipts_rlp(receipts: &[Vec<u8>]) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new_list(receipts.len());
    for receipt in receipts {
        stream.append_raw(receipt, 1);
    }
    stream.out().to_vec()
}

fn block_log_bloom<'a>(logs: impl Iterator<Item = &'a FinalChainEvmLog>) -> FinalChainLogBloom {
    let mut bloom = FinalChainLogBloom::ZERO;
    for log in logs {
        add_bloom_value(&mut bloom, &log.address);
        for topic in &log.topics {
            add_bloom_value(&mut bloom, &topic.topic);
        }
    }
    bloom
}

fn add_bloom_value(bloom: &mut FinalChainLogBloom, value: &[u8]) {
    use tiny_keccak::{Hasher, Keccak};

    let mut hash = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(value);
    hasher.finalize(&mut hash);

    for offset in [0usize, 2, 4] {
        let bit = (((hash[offset] as usize) << 8) | hash[offset + 1] as usize) & 2047;
        let byte_index = rustaxa_types::FINAL_CHAIN_LOG_BLOOM_BYTES - 1 - (bit / 8);
        bloom.as_mut_bytes()[byte_index] |= 1u8 << (bit % 8);
    }
}

fn ordered_root<'a>(values: impl Iterator<Item = &'a [u8]>) -> H256 {
    H256::from_slice(ordered_trie_root::<KeccakHasher, _>(values).as_ref())
}

fn u256_from_big_endian(bytes: &[u8]) -> U256 {
    U256::from_big_endian(bytes)
}

fn u256_to_big_endian(value: U256) -> Vec<u8> {
    if value.is_zero() {
        return vec![0];
    }
    let bytes = value.to_big_endian();
    let first_nonzero = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len() - 1);
    bytes[first_nonzero..].to_vec()
}

pub(crate) fn u256_to_nonce_bytes(value: U256) -> Vec<u8> {
    if value.is_zero() {
        Vec::new()
    } else {
        u256_to_big_endian(value)
    }
}

fn h256_from_slice(bytes: &[u8], field: &str) -> Result<H256, anyhow::Error> {
    if bytes.len() != 32 {
        anyhow::bail!("{field} must be 32 bytes, got {}", bytes.len());
    }
    Ok(H256::from_slice(bytes))
}

fn encode_system_transaction_hashes_rlp(hashes: impl Iterator<Item = [u8; 32]>) -> Vec<u8> {
    let hashes = hashes.collect::<Vec<_>>();
    let mut stream = rlp::RlpStream::new_list(hashes.len());
    for hash in hashes {
        stream.append(&hash.as_slice());
    }
    stream.out().to_vec()
}

pub(crate) fn final_chain_external_evm_publication_plan_id(
    plan: &FinalChainExternalEvmPublicationPlan,
) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};

    let mut hasher = Keccak::v256();
    hasher.update(b"rustaxa-final-chain-external-evm-publication-plan-v3");
    hasher.update(&plan.request_id);
    hasher.update(&plan.period.as_u64().to_be_bytes());
    hasher.update(&plan.block_hash);
    hash_bytes_with_len(&mut hasher, &plan.block_header_rlp);
    hash_bytes_with_len(&mut hasher, &plan.stored_header_rlp);
    hash_bytes_with_len(&mut hasher, &plan.receipts_rlp);
    hash_bytes_with_len(&mut hasher, plan.indexed_log_bloom.as_ref());
    hash_bytes_with_len(&mut hasher, &plan.system_transaction_hashes_rlp);
    hasher.update(&(plan.transaction_publications.len() as u64).to_be_bytes());
    for publication in &plan.transaction_publications {
        hasher.update(&publication.transaction_hash);
        hasher.update(&publication.position.as_u32().to_be_bytes());
        hasher.update(&[u8::from(publication.is_system)]);
        if !publication.transaction_rlp.is_empty() {
            hash_bytes_with_len(&mut hasher, &publication.transaction_rlp);
        }
        hash_bytes_with_len(&mut hasher, &publication.receipt_rlp);
    }
    hasher.update(&plan.executed_dag_blocks.to_be_bytes());
    hasher.update(&plan.executed_transactions.to_be_bytes());
    if plan.proposal_period_dag_level_update.has_update {
        hasher.update(b"proposal-period-dag-level");
        hasher.update(&plan.proposal_period_dag_level_update.level.to_be_bytes());
    }
    hasher.update(
        &plan
            .rewards_stats_update
            .current_period
            .as_u64()
            .to_be_bytes(),
    );
    hasher.update(&[u8::from(plan.rewards_stats_update.cache_current_period)]);
    hasher.update(&[u8::from(plan.rewards_stats_update.clear_cached_stats)]);
    hash_bytes_with_len(
        &mut hasher,
        &plan.rewards_stats_update.current_block_stats_rlp,
    );
    hash_bytes_with_len(&mut hasher, &plan.dpos_snapshot_rlp);
    hash_bytes_with_len(&mut hasher, &plan.account_snapshot_rlp);
    hash_bytes_with_len(&mut hasher, &plan.concrete_marker_rlp);
    hash_bytes_with_len(&mut hasher, &plan.concrete_projection_rlp);
    hasher.update(&plan.concrete_projection_hash);
    hash_bytes_with_len(&mut hasher, &plan.concrete_provenance_rlp);

    let mut plan_id = [0u8; 32];
    hasher.finalize(&mut plan_id);
    plan_id
}

pub(crate) fn final_chain_external_evm_commit_decision_id(
    request_id: [u8; 32],
    plan_id: [u8; 32],
    period: FinalChainBlockNumber,
    publication_block_hash: [u8; 32],
) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};

    let mut hasher = Keccak::v256();
    hasher.update(b"rustaxa-final-chain-external-evm-commit-decision-v1");
    hasher.update(&request_id);
    hasher.update(&plan_id);
    hasher.update(&period.as_u64().to_be_bytes());
    hasher.update(&publication_block_hash);

    let mut decision_id = [0u8; 32];
    hasher.finalize(&mut decision_id);
    decision_id
}

/// Derives the durable identity for one exact concrete-state lifecycle.
///
/// Unlike a publication plan id, this identity also covers the prior,
/// post-transaction, and post-rewards roots. It is therefore suitable for
/// correlating a pending marker with restart recovery observations.
pub(crate) fn final_chain_external_evm_lifecycle_id(
    request_id: [u8; 32],
    plan_id: [u8; 32],
    period: FinalChainBlockNumber,
    publication_block_hash: [u8; 32],
    prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    post_transaction_state_root: [u8; 32],
    post_rewards_state_root: [u8; 32],
) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};

    let mut hasher = Keccak::v256();
    hasher.update(b"rustaxa-final-chain-external-evm-lifecycle-v1");
    hasher.update(&request_id);
    hasher.update(&plan_id);
    hasher.update(&period.as_u64().to_be_bytes());
    hasher.update(&publication_block_hash);
    hasher.update(&prior_state.period.as_u64().to_be_bytes());
    hasher.update(&prior_state.state_root);
    hasher.update(&post_transaction_state_root);
    hasher.update(&post_rewards_state_root);
    let mut lifecycle_id = [0; 32];
    hasher.finalize(&mut lifecycle_id);
    lifecycle_id
}

/// Validates durable marker, FinalChain, and concrete-state facts without
/// mutating either persistence owner.
///
/// Normal live recovery returns `READY_TO_PUBLISH`. An exact duplicate block
/// returns `ALREADY_PUBLISHED`, while a concrete descriptor still equal to the
/// exact prior state proves the staged commit was not durable and returns
/// `CLEAR_UNCOMMITTED`. Every gap, ahead descriptor, root mismatch, stale
/// identity, missing observation, or ambiguous outcome is rejected and must
/// leave the marker intact.
pub fn validate_external_evm_recovery_fact(
    fact: &FinalChainExternalEvmRecoveryFact,
) -> FinalChainExternalEvmRecoveryDecision {
    let decision = |status, error_code: &str| FinalChainExternalEvmRecoveryDecision {
        lifecycle_id: fact.lifecycle_id,
        request_id: fact.request_id,
        plan_id: fact.plan_id,
        period: fact.period,
        publication_block_hash: fact.publication_block_hash,
        status,
        error_code: error_code.to_string(),
    };
    if fact.request_id == [0; 32]
        || fact.plan_id == [0; 32]
        || fact.publication_block_hash == [0; 32]
        || fact.post_transaction_state_root == [0; 32]
        || fact.post_rewards_state_root == [0; 32]
        || fact.prior_state.state_root == [0; 32]
    {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_IDENTITY_OR_ROOT_MISSING",
        );
    }
    let Some(expected_period) = fact.prior_state.period.checked_next() else {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_PRIOR_PERIOD_OVERFLOW",
        );
    };
    if fact.period != expected_period {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_PERIOD_GAP",
        );
    }
    let expected_marker = match decode_concrete_execution_marker(&fact.expected_concrete_marker_rlp)
    {
        Ok(marker) => marker,
        Err(_) => {
            return decision(
                FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
                "FINAL_CHAIN_EVM_RECOVERY_CONCRETE_MARKER_INVALID",
            );
        }
    };
    let expected_provenance =
        match decode_concrete_state_provenance(&fact.expected_concrete_provenance_rlp) {
            Ok(provenance) => provenance,
            Err(_) => {
                return decision(
                    FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
                    "FINAL_CHAIN_EVM_RECOVERY_EXPECTED_PROVENANCE_INVALID",
                );
            }
        };
    let observed_provenance =
        match decode_concrete_state_provenance(&fact.observed_concrete_provenance_rlp) {
            Ok(provenance) => provenance,
            Err(_) => {
                return decision(
                    FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
                    "FINAL_CHAIN_EVM_RECOVERY_OBSERVED_PROVENANCE_INVALID",
                );
            }
        };
    if expected_marker.period != fact.period.as_u64()
        || expected_marker.prior_state.period != fact.prior_state.period.as_u64()
        || expected_marker.prior_state.root != fact.prior_state.state_root
        || expected_provenance.identity != expected_marker.identity
        || expected_provenance.generation != expected_marker.generation
        || expected_provenance.plan_hash != expected_marker.plan_hash
        || expected_provenance.committed_state.period != fact.period.as_u64()
        || expected_provenance.committed_state.root != fact.post_rewards_state_root
    {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_CONCRETE_PLAN_MISMATCH",
        );
    }
    if observed_provenance.identity != expected_provenance.identity {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_PROVENANCE_IDENTITY_MISMATCH",
        );
    }
    if !fact.pending_concrete_marker_rlp.is_empty()
        && fact.pending_concrete_marker_rlp != fact.expected_concrete_marker_rlp
    {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_PENDING_CONCRETE_MARKER_MISMATCH",
        );
    }
    let expected_lifecycle_id = final_chain_external_evm_lifecycle_id(
        fact.request_id,
        fact.plan_id,
        fact.period,
        fact.publication_block_hash,
        fact.prior_state,
        fact.post_transaction_state_root,
        fact.post_rewards_state_root,
    );
    if fact.lifecycle_id != expected_lifecycle_id {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_LIFECYCLE_ID_MISMATCH",
        );
    }

    if let Some(finalized_block_hash) = fact.finalized_block_hash {
        if finalized_block_hash != fact.publication_block_hash {
            return decision(
                FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
                "FINAL_CHAIN_EVM_RECOVERY_EXISTING_BLOCK_HASH_MISMATCH",
            );
        }
        let expected_published_state = FinalChainExternalEvmCommittedStateDescriptor {
            period: fact.period,
            state_root: fact.post_rewards_state_root,
        };
        if fact.finalized_block_state != Some(expected_published_state) {
            return decision(
                FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
                "FINAL_CHAIN_EVM_RECOVERY_EXISTING_BLOCK_ROOT_MISMATCH",
            );
        }
        if fact.finalized_head.period < fact.period {
            return decision(
                FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
                "FINAL_CHAIN_EVM_RECOVERY_EXISTING_BLOCK_AHEAD_OF_HEAD",
            );
        }
        if fact.committed_state != Some(expected_published_state) {
            return decision(
                FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
                "FINAL_CHAIN_EVM_RECOVERY_DUPLICATE_COMMITTED_DESCRIPTOR_MISMATCH",
            );
        }
        if !fact.pending_concrete_marker_rlp.is_empty() {
            return decision(
                FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
                "FINAL_CHAIN_EVM_RECOVERY_PUBLISHED_WITH_PENDING_CONCRETE_MARKER",
            );
        }
        if fact.observed_concrete_provenance_rlp != fact.expected_concrete_provenance_rlp {
            return decision(
                FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
                "FINAL_CHAIN_EVM_RECOVERY_PUBLISHED_PROVENANCE_MISMATCH",
            );
        }
        return decision(FINAL_CHAIN_EVM_RECOVERY_DECISION_ALREADY_PUBLISHED, "");
    }
    if fact.finalized_block_state.is_some() {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_BLOCK_STATE_WITHOUT_HASH",
        );
    }
    if fact.finalized_head.period != fact.prior_state.period {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            if fact.finalized_head.period > fact.prior_state.period {
                "FINAL_CHAIN_EVM_RECOVERY_STALE_MARKER_OR_HEAD_AHEAD"
            } else {
                "FINAL_CHAIN_EVM_RECOVERY_FINALIZED_HEAD_BEHIND"
            },
        );
    }
    if fact.finalized_head.state_root != fact.prior_state.state_root {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_FINALIZED_PRIOR_ROOT_MISMATCH",
        );
    }
    let Some(committed_state) = fact.committed_state else {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_COMMITTED_DESCRIPTOR_MISSING",
        );
    };
    if committed_state == fact.prior_state {
        if observed_provenance.committed_state.period != fact.prior_state.period.as_u64()
            || observed_provenance.committed_state.root != fact.prior_state.state_root
            || observed_provenance.generation.checked_add(1) != Some(expected_provenance.generation)
        {
            return decision(
                FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
                "FINAL_CHAIN_EVM_RECOVERY_PRIOR_PROVENANCE_MISMATCH",
            );
        }
        return decision(FINAL_CHAIN_EVM_RECOVERY_DECISION_CLEAR_UNCOMMITTED, "");
    }
    if committed_state.period < fact.prior_state.period {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_COMMITTED_STATE_BEHIND",
        );
    }
    if committed_state.period == fact.prior_state.period {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_COMMITTED_PRIOR_ROOT_MISMATCH",
        );
    }
    if committed_state.period > fact.period {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_COMMITTED_STATE_AHEAD",
        );
    }
    if committed_state.period != fact.period {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_COMMITTED_PERIOD_GAP",
        );
    }
    if committed_state.state_root != fact.post_rewards_state_root {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_COMMITTED_ROOT_MISMATCH",
        );
    }
    if !fact.pending_concrete_marker_rlp.is_empty() {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_COMMITTED_WITH_PENDING_CONCRETE_MARKER",
        );
    }
    if fact.observed_concrete_provenance_rlp != fact.expected_concrete_provenance_rlp {
        return decision(
            FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED,
            "FINAL_CHAIN_EVM_RECOVERY_COMMITTED_PROVENANCE_MISMATCH",
        );
    }
    decision(FINAL_CHAIN_EVM_RECOVERY_DECISION_READY_TO_PUBLISH, "")
}

/// Materializes the exact StateAPI discard instruction for a validated
/// uncommitted recovery fact.
///
/// The executor must treat every field as opaque/correlated input, execute the
/// marker discard once, verify reopen at `prior_state`, and retry recovery with
/// a fresh provenance/pending-marker snapshot. C++ must not decode or derive
/// any part of this instruction.
pub fn external_evm_recovery_discard_request(
    fact: &FinalChainExternalEvmRecoveryFact,
    decision: &FinalChainExternalEvmRecoveryDecision,
) -> Result<FinalChainExternalEvmDiscardRequest, anyhow::Error> {
    ensure!(
        decision.status == FINAL_CHAIN_EVM_RECOVERY_DECISION_CLEAR_UNCOMMITTED
            && decision.lifecycle_id == fact.lifecycle_id
            && decision.request_id == fact.request_id
            && decision.plan_id == fact.plan_id
            && decision.period == fact.period
            && decision.publication_block_hash == fact.publication_block_hash
            && decision.error_code.is_empty(),
        "FINAL_CHAIN_EVM_RECOVERY_DISCARD_DECISION_MISMATCH"
    );
    ensure!(
        !fact.pending_concrete_marker_rlp.is_empty()
            && fact.pending_concrete_marker_rlp == fact.expected_concrete_marker_rlp,
        "FINAL_CHAIN_EVM_RECOVERY_DISCARD_MARKER_MISSING"
    );
    let marker = decode_concrete_execution_marker(&fact.pending_concrete_marker_rlp)
        .context("FINAL_CHAIN_EVM_RECOVERY_DISCARD_MARKER_INVALID")?;
    ensure!(
        marker.period == fact.period.as_u64()
            && marker.prior_state.period == fact.prior_state.period.as_u64()
            && marker.prior_state.root == fact.prior_state.state_root,
        "FINAL_CHAIN_EVM_RECOVERY_DISCARD_MARKER_LINEAGE_MISMATCH"
    );
    Ok(FinalChainExternalEvmDiscardRequest {
        request_id: fact.request_id,
        period: fact.period,
        concrete_marker_rlp: fact.pending_concrete_marker_rlp.clone(),
        marker_hash: concrete_state_bytes_digest(&fact.pending_concrete_marker_rlp),
        prior_state: fact.prior_state,
    })
}

fn hash_bytes_with_len(hasher: &mut impl tiny_keccak::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn concrete_transactions_hash(transactions: &[FinalChainEvmTransactionInput]) -> [u8; 32] {
    let mut stream = rlp::RlpStream::new_list(transactions.len());
    for transaction in transactions {
        stream.append(&transaction.rlp);
    }
    concrete_state_bytes_digest(&stream.out())
}

fn concrete_rewards_plan_hash(
    request: &FinalChainExecutionRequest,
    evm_request: &FinalChainEvmExecutionRequest,
) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};

    let mut hasher = Keccak::v256();
    hasher.update(b"rustaxa-final-chain-concrete-rewards-plan-v1");
    hasher.update(&evm_request.request_id);
    hasher.update(&request.blocks_per_year.to_be_bytes());
    hasher.update(&(request.finalized_dag_blocks.len() as u64).to_be_bytes());
    for block in &request.finalized_dag_blocks {
        hasher.update(&block.author);
        hasher.update(&block.difficulty.to_be_bytes());
        for transaction_hash in &block.transaction_hashes {
            hasher.update(transaction_hash);
        }
    }
    for vote in &request.cert_votes {
        hasher.update(&vote.period.to_be_bytes());
        hasher.update(vote.voter.as_bytes());
        hasher.update(&vote.weight.to_be_bytes());
    }
    let mut hash = [0; 32];
    hasher.finalize(&mut hash);
    hash
}

fn concrete_execution_plan_hash(
    request_id: [u8; 32],
    period: FinalChainBlockNumber,
    prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    transactions_hash: [u8; 32],
    rewards_hash: [u8; 32],
) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};

    let mut hasher = Keccak::v256();
    hasher.update(b"rustaxa-final-chain-concrete-execution-plan-v1");
    hasher.update(&request_id);
    hasher.update(&period.as_u64().to_be_bytes());
    hasher.update(&prior_state.period.as_u64().to_be_bytes());
    hasher.update(&prior_state.state_root);
    hasher.update(&transactions_hash);
    hasher.update(&rewards_hash);
    let mut hash = [0; 32];
    hasher.finalize(&mut hash);
    hash
}

fn commit_plan_post_transaction_root(
    session: &FinalChainExecutionSession,
) -> Result<[u8; 32], anyhow::Error> {
    session
        .external_evm_commit_plan
        .as_ref()
        .map(|plan| plan.post_transaction_state_root)
        .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_CONCRETE_COMMIT_PLAN_MISSING"))
}

fn commit_plan_post_rewards_root(
    session: &FinalChainExecutionSession,
) -> Result<[u8; 32], anyhow::Error> {
    session
        .external_evm_commit_plan
        .as_ref()
        .map(|plan| plan.post_rewards_state_root)
        .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_CONCRETE_COMMIT_PLAN_MISSING"))
}

fn encode_concrete_rewards_input(stats: &[RewardsStatsPeriodRlp]) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new_list(stats.len());
    for stat in stats {
        stream.append_raw(&stat.data, 1);
    }
    stream.out().to_vec()
}

fn validate_regular_transaction_count(count: usize) -> Result<(), &'static str> {
    u32::try_from(count)
        .map(|_| ())
        .map_err(|_| "FINAL_CHAIN_REGULAR_TRANSACTION_COUNT_EXCEEDS_U32")
}

fn validate_combined_transaction_count(
    regular_count: usize,
    system_count: usize,
) -> Result<(), &'static str> {
    let count = regular_count
        .checked_add(system_count)
        .ok_or("FINAL_CHAIN_COMBINED_TRANSACTION_COUNT_EXCEEDS_U32")?;
    u32::try_from(count)
        .map(|_| ())
        .map_err(|_| "FINAL_CHAIN_COMBINED_TRANSACTION_COUNT_EXCEEDS_U32")
}

fn classify_ordered_execution_transactions(
    transactions: &[FinalizationTransaction],
) -> Result<Vec<FinalChainEvmTransactionInput>, anyhow::Error> {
    transactions
        .iter()
        .enumerate()
        .map(|(position, transaction)| {
            let kind = transaction_kind(transaction);
            Ok(FinalChainEvmTransactionInput {
                position: FinalChainTransactionPosition::try_from(position).map_err(|_| {
                    anyhow::anyhow!("FINAL_CHAIN_REGULAR_TRANSACTION_COUNT_EXCEEDS_U32")
                })?,
                hash: transaction.hash,
                sender: transaction.sender,
                receiver: transaction.receiver,
                nonce: transaction.nonce.clone(),
                value: transaction.value,
                gas_price: transaction.gas_price,
                gas_limit: transaction.gas_limit,
                data: transaction.data.clone(),
                rlp: transaction.rlp.clone(),
                kind,
                is_system: false,
            })
        })
        .collect()
}

fn count_external_evm_transactions(transactions: &[FinalChainEvmTransactionInput]) -> u64 {
    transactions
        .iter()
        .filter(|transaction| {
            matches!(
                transaction.kind,
                FINAL_CHAIN_EXECUTION_TX_KIND_EXTERNAL_EVM_CALL
                    | FINAL_CHAIN_EXECUTION_TX_KIND_EXTERNAL_EVM_CREATE
            )
        })
        .count() as u64
}

fn transaction_kind(transaction: &FinalizationTransaction) -> u8 {
    if transaction.receiver == Some(DPOS_CONTRACT_ADDRESS) {
        FINAL_CHAIN_EXECUTION_TX_KIND_DPOS_CONTRACT
    } else if transaction.receiver == Some(SLASHING_CONTRACT_ADDRESS) {
        FINAL_CHAIN_EXECUTION_TX_KIND_SLASHING_CONTRACT
    } else if transaction.receiver.is_none() {
        FINAL_CHAIN_EXECUTION_TX_KIND_EXTERNAL_EVM_CREATE
    } else if !transaction.data.is_empty() {
        FINAL_CHAIN_EXECUTION_TX_KIND_EXTERNAL_EVM_CALL
    } else {
        FINAL_CHAIN_EXECUTION_TX_KIND_NATIVE_VALUE_TRANSFER
    }
}

fn system_transaction_request_id(
    block_number: FinalChainBlockNumber,
    metadata: &rustaxa_types::PbftBlockMetadata,
    block_gas_limit: FinalChainGas,
    transactions: &[FinalChainEvmTransactionInput],
) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};

    let mut hasher = Keccak::v256();
    hasher.update(b"rustaxa-final-chain-system-transactions");
    hasher.update(&block_number.as_u64().to_be_bytes());
    hasher.update(metadata.author.as_bytes());
    hasher.update(&metadata.timestamp.to_be_bytes());
    hasher.update(&block_gas_limit.as_u64().to_be_bytes());
    for transaction in transactions {
        hasher.update(&u64::from(transaction.position.as_u32()).to_be_bytes());
        hasher.update(&transaction.hash);
        hasher.update(&[transaction.kind]);
    }
    let mut request_id = [0u8; 32];
    hasher.finalize(&mut request_id);
    request_id
}

fn execution_request_id(
    block_number: FinalChainBlockNumber,
    metadata: &rustaxa_types::PbftBlockMetadata,
    block_gas_limit: FinalChainGas,
    prior_state: FinalChainExternalEvmCommittedStateDescriptor,
    transactions: &[FinalChainEvmTransactionInput],
) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};

    let mut hasher = Keccak::v256();
    // Version the identity domain because the exact concrete prior descriptor
    // is now consensus-critical and prevents cross-root replay.
    hasher.update(b"rustaxa-final-chain-evm-execution-v3");
    hasher.update(&block_number.as_u64().to_be_bytes());
    hasher.update(&prior_state.period.as_u64().to_be_bytes());
    hasher.update(&prior_state.state_root);
    hasher.update(metadata.author.as_bytes());
    hasher.update(&metadata.timestamp.to_be_bytes());
    hasher.update(&block_gas_limit.as_u64().to_be_bytes());
    for transaction in transactions {
        hasher.update(&u64::from(transaction.position.as_u32()).to_be_bytes());
        hasher.update(&transaction.hash);
        hasher.update(&transaction.sender);
        match transaction.receiver {
            Some(receiver) => {
                hasher.update(&[1]);
                hasher.update(&receiver);
            }
            None => hasher.update(&[0]),
        }
        let nonce_bytes = transaction.nonce.to_bytes();
        hasher.update(&(nonce_bytes.len() as u32).to_be_bytes());
        hasher.update(&nonce_bytes);
        if transaction.is_system {
            hasher.update(&transaction.value.to_legacy_minimal_bytes());
        } else {
            hasher.update(&transaction.value.to_fixed_be_bytes());
        }
        if transaction.is_system {
            hasher.update(&u256_to_big_endian(transaction.gas_price.as_u256()));
        } else {
            hasher.update(&transaction.gas_price.to_fixed_be_bytes());
        }
        hasher.update(&transaction.gas_limit.as_u64().to_be_bytes());
        hasher.update(&transaction.data);
        hasher.update(&transaction.rlp);
        hasher.update(&[transaction.kind]);
        hasher.update(&[u8::from(transaction.is_system)]);
    }
    let mut request_id = [0u8; 32];
    hasher.finalize(&mut request_id);
    request_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::concrete_state_projection::{
        FinalChainConcreteIdentity, FinalChainConcreteStateProjection,
        FinalChainConcreteTransactionEffect, concrete_storage_catalog_hash,
        encode_concrete_state_projection,
    };
    use ethereum_types::H256;
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;

    fn request_with_transactions(
        transactions: Vec<FinalizationTransaction>,
        mode: u8,
    ) -> FinalChainExecutionRequest {
        FinalChainExecutionRequest {
            pbft_block_rlp: invalid_pbft_rlp(),
            transactions,
            finalized_dag_blocks: Vec::new(),
            blocks_per_year: 0,
            cert_votes: Vec::new(),
            block_gas_limit: 1_000_000.into(),
            mode,
        }
    }

    fn valid_request(
        transactions: Vec<FinalizationTransaction>,
        mode: u8,
    ) -> FinalChainExecutionRequest {
        let mut request = request_with_transactions(transactions, mode);
        request.pbft_block_rlp = signed_pbft_block_rlp(7);
        request
    }

    fn transaction(
        hash_byte: u8,
        receiver: Option<[u8; 20]>,
        data: Vec<u8>,
    ) -> FinalizationTransaction {
        FinalizationTransaction {
            hash: [hash_byte; 32],
            sender: [1; 20],
            receiver,
            nonce: FinalChainNonce::zero(),
            value: U256::zero().into(),
            gas_price: U256::zero().into(),
            gas_limit: 21_000.into(),
            data,
            rlp: vec![hash_byte],
        }
    }

    fn evm_result(
        tx: &FinalChainEvmTransactionInput,
        status: u8,
        gas_used: u64,
        cumulative_gas_used: u64,
        receipt_rlp: Vec<u8>,
    ) -> FinalChainEvmTransactionResult {
        FinalChainEvmTransactionResult {
            position: tx.position,
            hash: tx.hash,
            status,
            gas_used: gas_used.into(),
            cumulative_gas_used: cumulative_gas_used.into(),
            receipt_rlp,
            logs: vec![FinalChainEvmLog {
                address: [0x44; 20],
                topics: vec![FinalChainEvmLogTopic { topic: [0x55; 32] }],
                data: vec![0x66],
            }],
            new_contract_address: None,
            output: Vec::new(),
            code_error: String::new(),
            consensus_error: String::new(),
        }
    }

    fn evm_result_with_encoded_receipt(
        tx: &FinalChainEvmTransactionInput,
        status: u8,
        gas_used: u64,
        cumulative_gas_used: u64,
    ) -> FinalChainEvmTransactionResult {
        let mut result = evm_result(tx, status, gas_used, cumulative_gas_used, Vec::new());
        result.receipt_rlp = encode_external_evm_receipt(&result);
        result
    }

    fn encode_concrete_execution_result(result: &FinalChainEvmTransactionResult) -> Vec<u8> {
        let mut stream = RlpStream::new_list(6);
        stream.append(&result.output);
        stream.append(&result.new_contract_address.unwrap_or_default().as_slice());
        stream.begin_list(result.logs.len());
        for log in &result.logs {
            stream.begin_list(3);
            stream.append(&log.address.as_slice());
            stream.begin_list(log.topics.len());
            for topic in &log.topics {
                stream.append(&topic.topic.as_slice());
            }
            stream.append(&log.data);
        }
        stream.append(&result.gas_used.as_u64());
        stream.append(&result.code_error);
        stream.append(&result.consensus_error);
        stream.out().to_vec()
    }

    fn concrete_projection_for_report(
        request: &FinalChainEvmExecutionRequest,
        report: &FinalChainEvmExecutionReport,
        projected_results: &[FinalChainEvmTransactionResult],
        post_rewards_state_root: [u8; 32],
    ) -> Vec<u8> {
        let marker = decode_concrete_execution_marker(&request.concrete_marker_rlp).unwrap();
        let storage = Vec::new();
        encode_concrete_state_projection(&FinalChainConcreteStateProjection {
            identity: marker.identity,
            generation: marker.generation,
            plan_hash: marker.plan_hash,
            prior_state: marker.prior_state,
            post_transaction_state: FinalChainConcreteState {
                period: request.period.as_u64(),
                root: report.post_transaction_state_root,
            },
            post_rewards_state: FinalChainConcreteState {
                period: request.period.as_u64(),
                root: post_rewards_state_root,
            },
            transaction_effects: request
                .transactions
                .iter()
                .zip(projected_results)
                .enumerate()
                .map(
                    |(index, (transaction, result))| FinalChainConcreteTransactionEffect {
                        index: index as u64,
                        transaction_rlp: transaction.rlp.clone(),
                        execution_result_rlp: encode_concrete_execution_result(result),
                        intermediate_state: FinalChainConcreteState {
                            period: request.period.as_u64(),
                            root: report.post_transaction_state_root,
                        },
                        accounts: Vec::new(),
                        storage: Vec::new(),
                        invocations: Vec::new(),
                    },
                )
                .collect(),
            accounts: Vec::new(),
            storage,
            invocations: Vec::new(),
            rewards_input: Vec::new(),
            catalog_hash: concrete_storage_catalog_hash(&[]),
        })
    }

    fn concrete_evm_report_identity(
        request: &FinalChainEvmExecutionRequest,
    ) -> FinalChainEvmExecutionReport {
        FinalChainEvmExecutionReport {
            request_id: request.request_id,
            prior_state: request.prior_state,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            concrete_plan_hash: request.concrete_plan_hash,
            transactions_hash: request.transactions_hash,
            rewards_hash: request.rewards_hash,
            ..Default::default()
        }
    }

    fn provide_system_transactions(
        session: &mut FinalChainExecutionSession,
        transactions: Vec<Vec<u8>>,
    ) -> FinalChainExecutionStep {
        let request = final_chain_execution_session_next(session);
        assert_eq!(
            request.action,
            FINAL_CHAIN_EXECUTION_ACTION_PROVIDE_SYSTEM_TRANSACTIONS
        );
        let step = final_chain_execution_session_report_system_transactions(
            session,
            FinalChainSystemTransactionReport {
                request_id: request.system_transaction_request.request_id,
                period: request.period,
                transactions,
            },
        );
        if step.action == FINAL_CHAIN_EXECUTION_ACTION_EXECUTE_EXTERNAL_EVM {
            let mut bound = final_chain_execution_session_bind_external_evm_prior_state(
                session,
                test_prior_state(),
            )
            .unwrap();
            let marker = FinalChainConcreteExecutionMarker {
                identity: FinalChainConcreteIdentity {
                    policy_version: 1,
                    database_id: [0x31; 32],
                    chain_id: [0x32; 32],
                },
                generation: 1,
                plan_hash: [0x33; 32],
                period: bound.period.as_u64(),
                prior_state: FinalChainConcreteState {
                    period: bound.prior_state.period.as_u64(),
                    root: bound.prior_state.state_root,
                },
                transactions_hash: [0x34; 32],
                rewards_hash: [0x35; 32],
            };
            bound.concrete_marker_rlp = encode_concrete_execution_marker(&marker);
            bound.concrete_plan_hash = marker.plan_hash;
            bound.transactions_hash = marker.transactions_hash;
            bound.rewards_hash = marker.rewards_hash;
            session.evm_request = Some(bound);
            final_chain_execution_session_next(session)
        } else {
            step
        }
    }

    fn test_prior_state() -> FinalChainExternalEvmCommittedStateDescriptor {
        FinalChainExternalEvmCommittedStateDescriptor {
            period: 6.into(),
            state_root: [0x99; 32],
        }
    }

    fn system_transaction_rlp(nonce: u64) -> Vec<u8> {
        let mut stream = RlpStream::new_list(9);
        stream.append(&U256::from(nonce));
        stream.append(&U256::zero());
        stream.append(&0u64);
        let receiver = [0u8; 20];
        stream.append(&receiver.as_slice());
        stream.append(&U256::zero());
        stream.append(&Vec::<u8>::new());
        stream.append(&U256::from(1u64));
        stream.append(&U256::zero());
        stream.append(&U256::zero());
        stream.out().to_vec()
    }

    fn system_transaction_plan_fact() -> FinalChainSystemTransactionPlanFact {
        FinalChainSystemTransactionPlanFact {
            request_id: [0x42; 32],
            period: 9.into(),
            is_pillar_block_period: true,
            bridge_contract_address: [0x77; 20],
            bridge_contract_found: true,
            bridge_contract_has_code: true,
            should_finalize_epoch: true,
            system_account_nonce: FinalChainNonce::from_u64(4),
            block_gas_limit: 1_000_000.into(),
        }
    }

    fn external_evm_state_commit_session() -> (
        FinalChainExecutionSession,
        FinalChainExternalEvmCommitPlan,
        FinalChainExternalEvmPublicationPlan,
    ) {
        let mut session = create_final_chain_execution_session(valid_request(
            Vec::new(),
            FINAL_CHAIN_EXECUTION_MODE_NATIVE_ONLY,
        ));
        let identity = FinalChainConcreteIdentity {
            policy_version: 1,
            database_id: [0x61; 32],
            chain_id: [0x62; 32],
        };
        let marker = FinalChainConcreteExecutionMarker {
            identity,
            generation: 4,
            plan_hash: [0x63; 32],
            period: 7,
            prior_state: FinalChainConcreteState {
                period: 6,
                root: test_prior_state().state_root,
            },
            transactions_hash: [0x64; 32],
            rewards_hash: [0x65; 32],
        };
        let concrete_marker_rlp = encode_concrete_execution_marker(&marker);
        let concrete_projection_rlp = Vec::new();
        let concrete_projection_hash = concrete_state_bytes_digest(&concrete_projection_rlp);
        let concrete_provenance_rlp =
            encode_concrete_state_provenance(&FinalChainConcreteStateProvenance {
                identity,
                generation: marker.generation,
                plan_hash: marker.plan_hash,
                committed_state: FinalChainConcreteState {
                    period: 7,
                    root: [0x33; 32],
                },
                transactions_hash: marker.transactions_hash,
                rewards_hash: marker.rewards_hash,
                projection_hash: concrete_projection_hash,
                catalog_hash: [0x66; 32],
            });
        let commit_plan = FinalChainExternalEvmCommitPlan {
            request_id: [0x11; 32],
            period: 7.into(),
            prior_state: test_prior_state(),
            post_transaction_state_root: [0x22; 32],
            post_rewards_state_root: [0x33; 32],
            concrete_marker_rlp: concrete_marker_rlp.clone(),
            concrete_projection_rlp: concrete_projection_rlp.clone(),
            concrete_projection_hash,
            concrete_provenance_rlp: concrete_provenance_rlp.clone(),
            error_code: String::new(),
            ..Default::default()
        };
        let publication_plan = FinalChainExternalEvmPublicationPlan {
            request_id: commit_plan.request_id,
            plan_id: [0x44; 32],
            period: 7.into(),
            block_hash: [0x55; 32],
            concrete_marker_rlp,
            concrete_projection_rlp,
            concrete_projection_hash,
            concrete_provenance_rlp,
            error_code: String::new(),
            ..Default::default()
        };
        session.status = FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_LIFECYCLE;
        session.external_evm_commit_plan = Some(commit_plan.clone());
        session.external_evm_publication_plan = Some(publication_plan.clone());
        (session, commit_plan, publication_plan)
    }

    #[test]
    fn typed_bloom_preserves_external_evm_publication_plan_id() {
        let (_session, _commit_plan, publication_plan) = external_evm_state_commit_session();
        assert_eq!(
            final_chain_external_evm_publication_plan_id(&publication_plan),
            [
                71, 13, 54, 177, 27, 148, 170, 187, 63, 4, 38, 175, 50, 50, 174, 252, 61, 25, 189,
                147, 237, 60, 96, 145, 90, 126, 38, 159, 89, 33, 125, 225,
            ]
        );
    }

    fn state_commit_request(
        commit_plan: &FinalChainExternalEvmCommitPlan,
        publication_plan: &FinalChainExternalEvmPublicationPlan,
    ) -> FinalChainExternalEvmStateCommitRequest {
        FinalChainExternalEvmStateCommitRequest {
            request_id: publication_plan.request_id,
            plan_id: publication_plan.plan_id,
            period: publication_plan.period,
            prior_state: commit_plan.prior_state,
            post_transaction_state_root: commit_plan.post_transaction_state_root,
            post_rewards_state_root: commit_plan.post_rewards_state_root,
            publication_block_hash: publication_plan.block_hash,
            concrete_marker_rlp: commit_plan.concrete_marker_rlp.clone(),
            concrete_projection_rlp: commit_plan.concrete_projection_rlp.clone(),
            concrete_projection_hash: commit_plan.concrete_projection_hash,
            concrete_provenance_rlp: commit_plan.concrete_provenance_rlp.clone(),
        }
    }

    fn lifecycle_report(
        commit_plan: &FinalChainExternalEvmCommitPlan,
        publication_plan: &FinalChainExternalEvmPublicationPlan,
        status: u8,
        error_code: String,
    ) -> FinalChainExternalEvmLifecycleReport {
        FinalChainExternalEvmLifecycleReport {
            request_id: publication_plan.request_id,
            plan_id: publication_plan.plan_id,
            period: publication_plan.period,
            prior_state: commit_plan.prior_state,
            post_transaction_state_root: commit_plan.post_transaction_state_root,
            post_rewards_state_root: commit_plan.post_rewards_state_root,
            publication_block_hash: publication_plan.block_hash,
            committed_state: match status {
                FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED => {
                    Some(FinalChainExternalEvmCommittedStateDescriptor {
                        period: publication_plan.period,
                        state_root: commit_plan.post_rewards_state_root,
                    })
                }
                FINAL_CHAIN_EVM_LIFECYCLE_STATUS_DISCARDED => Some(commit_plan.prior_state),
                _ => None,
            },
            status,
            error_code,
        }
    }

    fn signed_pbft_block_rlp(period: u64) -> Vec<u8> {
        let signing_key = SigningKey::from_slice(&[9u8; 32]).unwrap();
        let timestamp = 1234u64;
        let mut unsigned_stream = RlpStream::new_list(7);
        append_pbft_block_fields(&mut unsigned_stream, period, timestamp);
        let message_hash = keccak256(&unsigned_stream.out());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(message_hash.as_bytes())
            .unwrap();
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(recovery_id.to_byte());

        let mut signed_stream = RlpStream::new_list(8);
        append_pbft_block_fields(&mut signed_stream, period, timestamp);
        signed_stream.append(&signature_bytes);
        signed_stream.out().to_vec()
    }

    fn invalid_pbft_rlp() -> Vec<u8> {
        vec![0xc0]
    }

    #[test]
    fn external_evm_system_transaction_planner_emits_finalize_epoch_rlp() {
        let fact = system_transaction_plan_fact();
        let plan = plan_external_evm_system_transactions(fact.clone()).unwrap();

        assert_eq!(plan.request_id, fact.request_id);
        assert_eq!(plan.period, fact.period);
        assert_eq!(plan.transactions.len(), 1);
        let envelope = LegacyTransactionEnvelope::decode_system(&plan.transactions[0]).unwrap();
        assert_eq!(
            envelope.sender,
            Some(H160::from(rustaxa_types::TARAXA_SYSTEM_ACCOUNT))
        );
        assert_eq!(
            envelope.nonce,
            U256::from_big_endian(&fact.system_account_nonce.to_bytes())
        );
        assert_eq!(envelope.gas_price, U256::zero());
        assert_eq!(envelope.gas, fact.block_gas_limit.as_u64());
        assert_eq!(
            envelope.receiver,
            Some(H160::from(fact.bridge_contract_address))
        );
        assert_eq!(envelope.value, U256::zero());
        assert_eq!(envelope.data, solidity_no_arg_call("finalizeEpoch()"));
        assert_eq!(envelope.chain_id, 0);
    }

    #[test]
    fn external_evm_system_transaction_planner_gates_empty_cases() {
        for mutate in [
            |fact: &mut FinalChainSystemTransactionPlanFact| fact.is_pillar_block_period = false,
            |fact: &mut FinalChainSystemTransactionPlanFact| {
                fact.bridge_contract_found = false;
                fact.bridge_contract_has_code = false;
            },
            |fact: &mut FinalChainSystemTransactionPlanFact| fact.bridge_contract_has_code = false,
            |fact: &mut FinalChainSystemTransactionPlanFact| fact.should_finalize_epoch = false,
        ] {
            let mut fact = system_transaction_plan_fact();
            mutate(&mut fact);
            let plan = plan_external_evm_system_transactions(fact).unwrap();
            assert!(plan.transactions.is_empty());
        }
    }

    #[test]
    fn external_evm_system_transaction_planner_rejects_nonce_above_u256() {
        let mut fact = system_transaction_plan_fact();
        fact.system_account_nonce = FinalChainNonce::from_bytes(&[0xff; 32]).unwrap().next();
        let err = plan_external_evm_system_transactions(fact).unwrap_err();
        assert_eq!(err.to_string(), "FINAL_CHAIN_SYSTEM_NONCE_EXCEEDS_U256");
    }

    #[test]
    fn execution_request_id_covers_fixed_gas_price_and_canonical_nonce_bytes() {
        let metadata = rustaxa_types::PbftBlockMetadata {
            author: H160::from_low_u64_be(1),
            period: 2u64,
            timestamp: 3,
            extra_data: Vec::new(),
        };
        let mut transaction = FinalChainEvmTransactionInput {
            position: 0.into(),
            hash: [4; 32],
            sender: [5; 20],
            receiver: None,
            nonce: FinalChainNonce::from_u64(u64::MAX),
            value: U256::zero().into(),
            gas_price: U256::zero().into(),
            gas_limit: 21_000.into(),
            data: Vec::new(),
            rlp: Vec::new(),
            kind: 0,
            is_system: false,
        };
        transaction.gas_price = FinalChainGasPrice::try_from(&[1][..]).unwrap();
        transaction.value = FinalChainTransactionValue::try_from(&[2][..]).unwrap();
        let legacy_width_id = execution_request_id(
            FinalChainBlockNumber::new(metadata.period),
            &metadata,
            1_000_000.into(),
            test_prior_state(),
            &[transaction.clone()],
        );
        let mut fixed_width = transaction.clone();
        let mut fixed_bytes = [0; 32];
        fixed_bytes[31] = 1;
        fixed_width.gas_price = FinalChainGasPrice::from_be_bytes(fixed_bytes);
        let mut fixed_value = [0; 32];
        fixed_value[31] = 2;
        fixed_width.value = FinalChainTransactionValue::try_from(fixed_value.as_slice()).unwrap();
        assert_eq!(
            legacy_width_id,
            execution_request_id(
                FinalChainBlockNumber::new(metadata.period),
                &metadata,
                1_000_000.into(),
                test_prior_state(),
                &[fixed_width.clone()],
            )
        );
        transaction.nonce = transaction.nonce.next();
        let widened_id = execution_request_id(
            FinalChainBlockNumber::new(metadata.period),
            &metadata,
            1_000_000.into(),
            test_prior_state(),
            &[transaction],
        );
        assert_ne!(legacy_width_id, widened_id);
        let mut other_prior = test_prior_state();
        other_prior.state_root[0] ^= 0xff;
        assert_ne!(
            legacy_width_id,
            execution_request_id(
                FinalChainBlockNumber::new(metadata.period),
                &metadata,
                1_000_000.into(),
                other_prior,
                &[fixed_width],
            )
        );
    }

    #[test]
    fn execution_request_id_preserves_minimal_zero_gas_system_preimage() {
        let metadata = rustaxa_types::PbftBlockMetadata {
            author: H160::from_low_u64_be(1),
            period: 2u64,
            timestamp: 3,
            extra_data: Vec::new(),
        };
        let transaction = FinalChainEvmTransactionInput {
            position: 0.into(),
            hash: [9; 32],
            sender: rustaxa_types::TARAXA_SYSTEM_ACCOUNT,
            receiver: Some([8; 20]),
            nonce: FinalChainNonce::from_u64(4),
            value: U256::zero().into(),
            gas_price: FinalChainGasPrice::zero(),
            gas_limit: 100_000.into(),
            data: vec![7, 6],
            rlp: vec![5],
            kind: FINAL_CHAIN_EXECUTION_TX_KIND_SYSTEM,
            is_system: true,
        };
        let request_id = execution_request_id(
            FinalChainBlockNumber::new(metadata.period),
            &metadata,
            1_000_000.into(),
            test_prior_state(),
            &[transaction],
        );
        assert_ne!(request_id, [0; 32]);
    }

    #[test]
    fn transaction_count_bounds_reject_before_external_session_actions() {
        assert!(validate_regular_transaction_count(u32::MAX as usize).is_ok());
        assert!(validate_combined_transaction_count(u32::MAX as usize, 0).is_ok());
        assert_eq!(
            validate_combined_transaction_count(u32::MAX as usize, 1),
            Err("FINAL_CHAIN_COMBINED_TRANSACTION_COUNT_EXCEEDS_U32")
        );
        assert_eq!(
            validate_combined_transaction_count(usize::MAX, 1),
            Err("FINAL_CHAIN_COMBINED_TRANSACTION_COUNT_EXCEEDS_U32")
        );

        let Ok(overflow_count) = usize::try_from(u64::from(u32::MAX) + 1) else {
            return;
        };
        assert_eq!(
            validate_regular_transaction_count(overflow_count),
            Err("FINAL_CHAIN_REGULAR_TRANSACTION_COUNT_EXCEEDS_U32")
        );

        let request =
            request_with_transactions(Vec::new(), FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED);
        let metadata = rustaxa_types::PbftBlockMetadata {
            author: H160::zero(),
            period: 1u64,
            timestamp: 2,
            extra_data: Vec::new(),
        };
        let block_number = FinalChainBlockNumber::new(metadata.period);
        let mut session = FinalChainExecutionSession::new_with_regular_transaction_count(
            request,
            metadata,
            block_number,
            overflow_count,
        );
        let step = final_chain_execution_session_next(&mut session);
        assert_eq!(step.action, FINAL_CHAIN_EXECUTION_ACTION_REJECT);
        assert_eq!(
            step.error_code,
            "FINAL_CHAIN_REGULAR_TRANSACTION_COUNT_EXCEEDS_U32"
        );
    }

    #[test]
    fn combined_transaction_count_overflow_rejects_before_execute_action() {
        let request = request_with_transactions(
            vec![transaction(1, Some([9; 20]), vec![0xaa])],
            FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
        );
        let metadata = rustaxa_types::PbftBlockMetadata {
            author: H160::zero(),
            period: 1u64,
            timestamp: 2,
            extra_data: Vec::new(),
        };
        let mut session = FinalChainExecutionSession::new(request, metadata);
        let provide_step = final_chain_execution_session_next(&mut session);
        assert_eq!(
            provide_step.action,
            FINAL_CHAIN_EXECUTION_ACTION_PROVIDE_SYSTEM_TRANSACTIONS
        );
        let report = FinalChainSystemTransactionReport {
            request_id: provide_step.system_transaction_request.request_id,
            period: provide_step.period,
            transactions: Vec::new(),
        };
        let rejected = final_chain_execution_session_report_system_transactions_with_count(
            &mut session,
            report,
            u32::MAX as usize,
        );
        assert_eq!(rejected.action, FINAL_CHAIN_EXECUTION_ACTION_REJECT);
        assert_ne!(
            rejected.action,
            FINAL_CHAIN_EXECUTION_ACTION_EXECUTE_EXTERNAL_EVM
        );
        assert_eq!(
            rejected.error_code,
            "FINAL_CHAIN_COMBINED_TRANSACTION_COUNT_EXCEEDS_U32"
        );
    }

    fn append_pbft_block_fields(stream: &mut RlpStream, period: u64, timestamp: u64) {
        stream.append(&H256::from_low_u64_be(10));
        stream.append(&H256::from_low_u64_be(11));
        stream.append(&H256::from_low_u64_be(12));
        stream.append(&H256::from_low_u64_be(13));
        stream.append(&period);
        stream.append(&timestamp);
        stream.begin_list(0);
    }

    fn keccak256(data: &[u8]) -> H256 {
        use tiny_keccak::{Hasher, Keccak};

        let mut hasher = Keccak::v256();
        hasher.update(data);
        let mut output = [0u8; 32];
        hasher.finalize(&mut output);
        H256::from(output)
    }

    #[test]
    fn native_supported_transactions_commit_in_rust() {
        let mut dpos_delegate_data = Vec::with_capacity(36);
        dpos_delegate_data.extend_from_slice(&[0x5c, 0x19, 0xa9, 0x5c]);
        dpos_delegate_data.extend_from_slice(&[0u8; 12]);
        dpos_delegate_data.extend_from_slice(&[7u8; 20]);

        let transactions = vec![
            transaction(1, Some([9; 20]), Vec::new()),
            transaction(2, Some(DPOS_CONTRACT_ADDRESS), dpos_delegate_data),
            transaction(3, Some(SLASHING_CONTRACT_ADDRESS), vec![4, 5, 6]),
        ];
        let mut session = create_final_chain_execution_session(valid_request(
            transactions,
            FINAL_CHAIN_EXECUTION_MODE_NATIVE_ONLY,
        ));

        let step = final_chain_execution_session_next(&mut session);

        assert_eq!(step.status, FINAL_CHAIN_EXECUTION_STATUS_READY);
        assert_eq!(step.action, FINAL_CHAIN_EXECUTION_ACTION_COMMIT_NATIVE);
        assert_eq!(step.period, 7.into());
        assert_eq!(step.external_evm_transaction_count, 0);
    }

    #[test]
    fn native_only_rejects_external_evm_transactions() {
        let transactions = vec![transaction(1, Some([9; 20]), vec![0xaa])];
        let mut session = create_final_chain_execution_session(valid_request(
            transactions,
            FINAL_CHAIN_EXECUTION_MODE_NATIVE_ONLY,
        ));

        let step = final_chain_execution_session_next(&mut session);

        assert_eq!(step.status, FINAL_CHAIN_EXECUTION_STATUS_REJECTED);
        assert_eq!(step.action, FINAL_CHAIN_EXECUTION_ACTION_REJECT);
        assert_eq!(
            step.error_code,
            "FINAL_CHAIN_EXECUTION_REQUIRES_EXTERNAL_EVM"
        );
    }

    #[test]
    fn external_evm_mode_transitions_even_an_empty_period_through_concrete_state() {
        let mut session = create_final_chain_execution_session(valid_request(
            Vec::new(),
            FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
        ));

        let system_step = final_chain_execution_session_next(&mut session);
        assert_eq!(
            system_step.action,
            FINAL_CHAIN_EXECUTION_ACTION_PROVIDE_SYSTEM_TRANSACTIONS
        );
        assert_eq!(system_step.system_transaction_request.period, 7.into());

        let step = provide_system_transactions(&mut session, Vec::new());
        assert_eq!(
            step.status,
            FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM
        );
        assert_eq!(
            step.action,
            FINAL_CHAIN_EXECUTION_ACTION_EXECUTE_EXTERNAL_EVM
        );
        assert_eq!(step.evm_request.prior_state, test_prior_state());
        assert!(step.evm_request.transactions.is_empty());
    }

    #[test]
    fn external_evm_mode_transitions_native_dpos_and_slashing_in_exact_order() {
        let transactions = vec![
            transaction(1, Some([9; 20]), Vec::new()),
            transaction(2, Some(DPOS_CONTRACT_ADDRESS), vec![1, 2, 3, 4]),
            transaction(3, Some(SLASHING_CONTRACT_ADDRESS), vec![5, 6, 7, 8]),
        ];
        let mut session = create_final_chain_execution_session(valid_request(
            transactions,
            FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
        ));
        let step = provide_system_transactions(&mut session, Vec::new());

        assert_eq!(
            step.action,
            FINAL_CHAIN_EXECUTION_ACTION_EXECUTE_EXTERNAL_EVM
        );
        assert_eq!(step.evm_request.prior_state, test_prior_state());
        assert_eq!(
            step.evm_request
                .transactions
                .iter()
                .map(|transaction| transaction.kind)
                .collect::<Vec<_>>(),
            vec![
                FINAL_CHAIN_EXECUTION_TX_KIND_NATIVE_VALUE_TRANSFER,
                FINAL_CHAIN_EXECUTION_TX_KIND_DPOS_CONTRACT,
                FINAL_CHAIN_EXECUTION_TX_KIND_SLASHING_CONTRACT,
            ]
        );
        assert_eq!(step.external_evm_transaction_count, 0);
    }

    #[test]
    fn external_evm_mode_builds_full_ordered_contract_call_request() {
        let transactions = vec![
            transaction(1, Some([9; 20]), Vec::new()),
            transaction(2, Some([8; 20]), vec![0xaa]),
            transaction(3, None, Vec::new()),
        ];
        let mut session = create_final_chain_execution_session(valid_request(
            transactions,
            FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
        ));

        let system_step = final_chain_execution_session_next(&mut session);

        assert_eq!(
            system_step.status,
            FINAL_CHAIN_EXECUTION_STATUS_WAITING_SYSTEM_TRANSACTIONS
        );
        assert_eq!(
            system_step.action,
            FINAL_CHAIN_EXECUTION_ACTION_PROVIDE_SYSTEM_TRANSACTIONS
        );
        assert_eq!(system_step.system_transaction_request.period, 7.into());
        assert_eq!(
            system_step
                .system_transaction_request
                .regular_transaction_count,
            3
        );
        let step = provide_system_transactions(&mut session, vec![system_transaction_rlp(4)]);
        assert_eq!(
            step.status,
            FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM
        );
        assert_eq!(
            step.action,
            FINAL_CHAIN_EXECUTION_ACTION_EXECUTE_EXTERNAL_EVM
        );
        assert_eq!(step.period, 7.into());
        assert_eq!(step.evm_request.block_gas_limit.as_u64(), 1_000_000);
        assert_eq!(step.external_evm_transaction_count, 2);
        assert_eq!(step.evm_request.transactions.len(), 4);
        assert_eq!(step.evm_request.transactions[0].position.as_u32(), 0);
        assert_eq!(
            step.evm_request.transactions[0].kind,
            FINAL_CHAIN_EXECUTION_TX_KIND_NATIVE_VALUE_TRANSFER
        );
        assert_eq!(step.evm_request.transactions[1].position.as_u32(), 1);
        assert_eq!(
            step.evm_request.transactions[1].kind,
            FINAL_CHAIN_EXECUTION_TX_KIND_EXTERNAL_EVM_CALL
        );
        assert_eq!(step.evm_request.transactions[2].position.as_u32(), 2);
        assert_eq!(
            step.evm_request.transactions[2].kind,
            FINAL_CHAIN_EXECUTION_TX_KIND_EXTERNAL_EVM_CREATE
        );
        assert_eq!(step.evm_request.transactions[3].position.as_u32(), 3);
        assert_eq!(
            step.evm_request.transactions[3].kind,
            FINAL_CHAIN_EXECUTION_TX_KIND_SYSTEM
        );
        assert!(step.evm_request.transactions[3].is_system);
    }

    #[test]
    fn evm_report_must_cover_full_ordered_transaction_request() {
        let transactions = vec![
            transaction(1, Some([9; 20]), Vec::new()),
            transaction(2, Some([8; 20]), vec![0xaa]),
        ];
        let mut session = create_final_chain_execution_session(valid_request(
            transactions,
            FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
        ));
        let step = provide_system_transactions(&mut session, Vec::new());
        let tx = step.evm_request.transactions[1].clone();
        let report = FinalChainEvmExecutionReport {
            request_id: step.evm_request.request_id,
            status: FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
            prior_state: step.evm_request.prior_state,
            concrete_marker_rlp: step.evm_request.concrete_marker_rlp.clone(),
            concrete_plan_hash: step.evm_request.concrete_plan_hash,
            transactions_hash: step.evm_request.transactions_hash,
            rewards_hash: step.evm_request.rewards_hash,
            post_transaction_state_root: [0x11; 32],
            cumulative_gas_used: 1.into(),
            results: vec![evm_result(&tx, 1, 1, 1, vec![0xc0])],
        };

        let rejected = final_chain_execution_session_report_evm(&mut session, report);

        assert_eq!(rejected.status, FINAL_CHAIN_EXECUTION_STATUS_REJECTED);
        assert_eq!(
            rejected.error_code,
            "FINAL_CHAIN_EVM_REPORT_RESULT_COUNT_MISMATCH"
        );
    }

    #[test]
    fn evm_report_identity_mismatch_is_rejected() {
        let transactions = vec![transaction(2, Some([8; 20]), vec![0xaa])];
        let mut session = create_final_chain_execution_session(valid_request(
            transactions,
            FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
        ));
        let step = provide_system_transactions(&mut session, Vec::new());
        let mut mismatched_result =
            evm_result(&step.evm_request.transactions[0], 1, 1, 1, vec![0xc0]);
        mismatched_result.hash = [0xff; 32];
        let mut report = FinalChainEvmExecutionReport {
            request_id: step.evm_request.request_id,
            status: FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
            prior_state: step.evm_request.prior_state,
            post_transaction_state_root: [0x11; 32],
            cumulative_gas_used: 1.into(),
            results: vec![mismatched_result],
            ..concrete_evm_report_identity(&step.evm_request)
        };

        let rejected = final_chain_execution_session_report_evm(&mut session, report.clone());
        assert_eq!(rejected.status, FINAL_CHAIN_EXECUTION_STATUS_REJECTED);
        assert_eq!(
            rejected.error_code,
            "FINAL_CHAIN_EVM_REPORT_TRANSACTION_MISMATCH"
        );

        report.request_id = [0xee; 32];
        let rejected = final_chain_execution_session_report_evm(&mut session, report);
        assert_eq!(rejected.status, FINAL_CHAIN_EXECUTION_STATUS_REJECTED);
    }

    #[test]
    fn successful_evm_report_requests_rewards_boundary() {
        let transactions = vec![transaction(2, Some([8; 20]), vec![0xaa])];
        let mut session = create_final_chain_execution_session(valid_request(
            transactions,
            FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
        ));
        let step = provide_system_transactions(&mut session, Vec::new());
        let tx = step.evm_request.transactions[0].clone();
        let report = FinalChainEvmExecutionReport {
            request_id: step.evm_request.request_id,
            status: FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
            prior_state: step.evm_request.prior_state,
            post_transaction_state_root: [0x11; 32],
            cumulative_gas_used: 1.into(),
            results: vec![evm_result_with_encoded_receipt(&tx, 1, 1, 1)],
            ..concrete_evm_report_identity(&step.evm_request)
        };

        let rewards = final_chain_execution_session_report_evm(&mut session, report);

        assert_eq!(
            rewards.status,
            FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_REWARDS
        );
        assert_eq!(
            rewards.action,
            FINAL_CHAIN_EXECUTION_ACTION_DISTRIBUTE_EXTERNAL_EVM_REWARDS
        );
        assert_eq!(
            rewards.evm_rewards_request.request_id,
            step.evm_request.request_id
        );
        assert_eq!(rewards.evm_rewards_request.period, 7.into());
        assert_eq!(rewards.evm_rewards_request.block_gas_used.as_u64(), 1);
        assert_eq!(
            rewards
                .evm_rewards_request
                .transaction_gas_used
                .iter()
                .map(|gas| gas.as_u64())
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(rewards.evm_rewards_request.transaction_fees, vec![vec![0]]);
        assert_eq!(
            rewards.evm_rewards_request.prior_state,
            step.evm_request.prior_state
        );
        assert_eq!(
            rewards.evm_rewards_request.post_transaction_state_root,
            [0x11; 32]
        );
    }

    #[test]
    fn evm_report_requires_exact_prior_and_post_transaction_roots() {
        for expected_error in [
            "FINAL_CHAIN_EVM_REPORT_PRIOR_STATE_MISMATCH",
            "FINAL_CHAIN_EVM_REPORT_POST_TRANSACTION_ROOT_MISSING",
        ] {
            let transactions = vec![transaction(2, Some([8; 20]), vec![0xaa])];
            let mut session = create_final_chain_execution_session(valid_request(
                transactions,
                FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
            ));
            let step = provide_system_transactions(&mut session, Vec::new());
            let tx = step.evm_request.transactions[0].clone();
            let mut report = FinalChainEvmExecutionReport {
                request_id: step.evm_request.request_id,
                status: FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
                prior_state: step.evm_request.prior_state,
                post_transaction_state_root: [0x11; 32],
                cumulative_gas_used: 1.into(),
                results: vec![evm_result_with_encoded_receipt(&tx, 1, 1, 1)],
                ..concrete_evm_report_identity(&step.evm_request)
            };
            if expected_error.ends_with("PRIOR_STATE_MISMATCH") {
                report.prior_state.state_root[0] ^= 0xff;
            } else {
                report.post_transaction_state_root = [0; 32];
            }

            let rejected = final_chain_execution_session_report_evm(&mut session, report);
            assert_eq!(rejected.status, FINAL_CHAIN_EXECUTION_STATUS_REJECTED);
            assert_eq!(rejected.error_code, expected_error);
        }
    }

    #[test]
    fn external_evm_rewards_plan_rejects_cross_session_identity() {
        let transactions = vec![transaction(2, Some([8; 20]), vec![0xaa])];
        let mut session = create_final_chain_execution_session(valid_request(
            transactions,
            FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
        ));
        let step = provide_system_transactions(&mut session, Vec::new());
        let tx = step.evm_request.transactions[0].clone();
        let _ = final_chain_execution_session_report_evm(
            &mut session,
            FinalChainEvmExecutionReport {
                request_id: step.evm_request.request_id,
                status: FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
                prior_state: step.evm_request.prior_state,
                post_transaction_state_root: [0x11; 32],
                cumulative_gas_used: 1.into(),
                results: vec![evm_result_with_encoded_receipt(&tx, 1, 1, 1)],
                ..concrete_evm_report_identity(&step.evm_request)
            },
        );
        session.prepared_rewards_stats_plan = Some(FinalChainPreparedExternalEvmRewardsStatsPlan {
            request_id: [0xff; 32],
            period: step.evm_request.period,
            expected_prior_head: step
                .evm_request
                .period
                .checked_sub_distance(1)
                .expect("final-chain rewards period has no prior head"),
            expected_runtime_generation: 0,
            distribution_stats: Vec::new(),
            storage_update: FinalChainExternalEvmRewardsStatsUpdate::default(),
        });

        assert_eq!(
            session_external_evm_rewards_stats_plan(&session)
                .unwrap_err()
                .to_string(),
            "FINAL_CHAIN_EVM_REWARDS_STATS_SESSION_MISMATCH"
        );
    }

    #[test]
    fn external_evm_rewards_report_builds_non_mutating_commit_plan() {
        let transactions = vec![
            transaction(1, Some([9; 20]), Vec::new()),
            transaction(2, Some([8; 20]), vec![0xaa]),
        ];
        let mut session = create_final_chain_execution_session(valid_request(
            transactions.clone(),
            FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
        ));
        let step = provide_system_transactions(&mut session, Vec::new());
        let first = evm_result_with_encoded_receipt(&step.evm_request.transactions[0], 1, 2, 2);
        let second = evm_result_with_encoded_receipt(&step.evm_request.transactions[1], 1, 3, 5);
        let evm_report = FinalChainEvmExecutionReport {
            request_id: step.evm_request.request_id,
            status: FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
            prior_state: step.evm_request.prior_state,
            post_transaction_state_root: [0x10; 32],
            cumulative_gas_used: 5.into(),
            results: vec![first.clone(), second.clone()],
            ..concrete_evm_report_identity(&step.evm_request)
        };
        let concrete_projection_rlp = concrete_projection_for_report(
            &step.evm_request,
            &evm_report,
            &evm_report.results,
            [0x22; 32],
        );
        let rewards = final_chain_execution_session_report_evm(&mut session, evm_report);
        assert_eq!(
            rewards.action,
            FINAL_CHAIN_EXECUTION_ACTION_DISTRIBUTE_EXTERNAL_EVM_REWARDS
        );

        let plan = final_chain_execution_session_plan_external_evm_commit(
            &mut session,
            FinalChainEvmRewardsReport {
                request_id: step.evm_request.request_id,
                period: 7.into(),
                status: FINAL_CHAIN_EVM_REWARDS_REPORT_STATUS_SUCCESS,
                prior_state: step.evm_request.prior_state,
                post_transaction_state_root: [0x10; 32],
                post_rewards_state_root: [0x22; 32],
                concrete_marker_rlp: step.evm_request.concrete_marker_rlp.clone(),
                concrete_plan_hash: step.evm_request.concrete_plan_hash,
                transactions_hash: step.evm_request.transactions_hash,
                rewards_hash: step.evm_request.rewards_hash,
                concrete_projection_hash: concrete_state_bytes_digest(&concrete_projection_rlp),
                concrete_projection_rlp,
                concrete_provenance_rlp: Vec::new(),
                total_reward: vec![0x33],
            },
        );

        assert!(plan.error_code.is_empty());
        assert_eq!(plan.period, 7.into());
        assert_eq!(plan.request_id, step.evm_request.request_id);
        assert_eq!(plan.prior_state, test_prior_state());
        assert_eq!(plan.post_transaction_state_root, [0x10; 32]);
        assert_eq!(plan.post_rewards_state_root, [0x22; 32]);
        assert_eq!(plan.total_reward, vec![0x33]);
        assert_eq!(plan.gas_used.as_u64(), 5);
        assert_eq!(plan.executed_dag_blocks, 0);
        assert_eq!(plan.executed_transactions, 2);
        assert_eq!(
            plan.transactions_root,
            <[u8; 32]>::from(ordered_root(
                transactions.iter().map(|tx| tx.rlp.as_slice())
            ))
        );
        assert_eq!(
            plan.receipts_root,
            <[u8; 32]>::from(ordered_root(
                [first.receipt_rlp.as_slice(), second.receipt_rlp.as_slice()].into_iter()
            ))
        );
        assert_eq!(
            plan.encoded_receipts,
            vec![first.receipt_rlp, second.receipt_rlp]
        );
        assert_eq!(
            plan.receipts_rlp,
            encode_receipts_rlp(&plan.encoded_receipts)
        );
        assert_eq!(plan.header_log_bloom.as_ref().len(), 256);
        assert_eq!(plan.indexed_log_bloom.as_ref().len(), 256);
        assert!(!plan.header_log_bloom.as_ref().iter().all(|byte| *byte == 0));
        assert_eq!(
            session.status,
            FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_PUBLICATION
        );
        assert!(session.error_code.is_empty());
    }

    #[test]
    fn arbitrary_evm_projection_result_must_match_host_report_before_header_planning() {
        let mut session = create_final_chain_execution_session(valid_request(
            vec![transaction(2, Some([8; 20]), vec![0xaa])],
            FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
        ));
        let step = provide_system_transactions(&mut session, Vec::new());
        assert_eq!(
            step.evm_request.transactions[0].kind,
            FINAL_CHAIN_EXECUTION_TX_KIND_EXTERNAL_EVM_CALL
        );
        let host_result =
            evm_result_with_encoded_receipt(&step.evm_request.transactions[0], 1, 3, 3);
        let evm_report = FinalChainEvmExecutionReport {
            request_id: step.evm_request.request_id,
            status: FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
            prior_state: step.evm_request.prior_state,
            post_transaction_state_root: [0x10; 32],
            cumulative_gas_used: 3.into(),
            results: vec![host_result.clone()],
            ..concrete_evm_report_identity(&step.evm_request)
        };
        let mut projected_result = host_result;
        projected_result.output = vec![0xde, 0xad];
        let concrete_projection_rlp = concrete_projection_for_report(
            &step.evm_request,
            &evm_report,
            &[projected_result],
            [0x22; 32],
        );
        let rewards = final_chain_execution_session_report_evm(&mut session, evm_report);
        assert_eq!(
            rewards.action,
            FINAL_CHAIN_EXECUTION_ACTION_DISTRIBUTE_EXTERNAL_EVM_REWARDS
        );

        let plan = final_chain_execution_session_plan_external_evm_commit(
            &mut session,
            FinalChainEvmRewardsReport {
                request_id: step.evm_request.request_id,
                period: step.evm_request.period,
                status: FINAL_CHAIN_EVM_REWARDS_REPORT_STATUS_SUCCESS,
                prior_state: step.evm_request.prior_state,
                post_transaction_state_root: [0x10; 32],
                post_rewards_state_root: [0x22; 32],
                concrete_marker_rlp: step.evm_request.concrete_marker_rlp.clone(),
                concrete_plan_hash: step.evm_request.concrete_plan_hash,
                transactions_hash: step.evm_request.transactions_hash,
                rewards_hash: step.evm_request.rewards_hash,
                concrete_projection_hash: concrete_state_bytes_digest(&concrete_projection_rlp),
                concrete_projection_rlp,
                concrete_provenance_rlp: Vec::new(),
                total_reward: Vec::new(),
            },
        );

        assert_eq!(session.status, FINAL_CHAIN_EXECUTION_STATUS_REJECTED);
        assert!(
            plan.error_code
                .contains("FINAL_CHAIN_CONCRETE_EXECUTION_RESULT_OUTPUT_MISMATCH"),
            "{}",
            plan.error_code
        );
        assert!(session.external_evm_commit_plan.is_none());
    }

    #[test]
    fn external_evm_state_commit_intent_must_match_rust_plan() {
        let (mut session, commit_plan, publication_plan) = external_evm_state_commit_session();
        let step = final_chain_execution_session_next(&mut session);
        assert_eq!(
            step.action,
            FINAL_CHAIN_EXECUTION_ACTION_REQUEST_EXTERNAL_EVM_STATE_COMMIT
        );
        let mut request = state_commit_request(&commit_plan, &publication_plan);
        request.plan_id[0] ^= 0xff;

        let intent =
            final_chain_execution_session_request_external_evm_state_commit(&mut session, request);

        assert_eq!(intent.status, FINAL_CHAIN_EVM_STATE_COMMIT_INTENT_REJECTED);
        assert_eq!(
            intent.error_code,
            "FINAL_CHAIN_EVM_STATE_COMMIT_PLAN_ID_MISMATCH"
        );
        assert_eq!(session.status, FINAL_CHAIN_EXECUTION_STATUS_REJECTED);
    }

    #[test]
    fn external_evm_lifecycle_requires_ready_state_commit_intent() {
        let (mut session, commit_plan, publication_plan) = external_evm_state_commit_session();

        let decision = final_chain_execution_session_report_external_evm_lifecycle(
            &mut session,
            lifecycle_report(
                &commit_plan,
                &publication_plan,
                FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED,
                String::new(),
            ),
        );

        assert_eq!(decision.status, FINAL_CHAIN_EVM_COMMIT_DECISION_REJECTED);
        assert_eq!(decision.error_code, "FINAL_CHAIN_EVM_LIFECYCLE_UNEXPECTED");
        assert_eq!(session.status, FINAL_CHAIN_EXECUTION_STATUS_REJECTED);
    }

    #[test]
    fn external_evm_committed_lifecycle_after_intent_can_publish() {
        let (mut session, commit_plan, publication_plan) = external_evm_state_commit_session();

        let intent = final_chain_execution_session_request_external_evm_state_commit(
            &mut session,
            state_commit_request(&commit_plan, &publication_plan),
        );
        assert_eq!(
            intent.status,
            FINAL_CHAIN_EVM_STATE_COMMIT_INTENT_READY_TO_COMMIT
        );
        assert_eq!(
            session.status,
            FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_STATE_COMMIT
        );
        let step = final_chain_execution_session_next(&mut session);
        assert_eq!(
            step.action,
            FINAL_CHAIN_EXECUTION_ACTION_REPORT_EXTERNAL_EVM_LIFECYCLE
        );

        let decision = final_chain_execution_session_report_external_evm_lifecycle(
            &mut session,
            lifecycle_report(
                &commit_plan,
                &publication_plan,
                FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED,
                String::new(),
            ),
        );

        assert_eq!(
            decision.status,
            FINAL_CHAIN_EVM_COMMIT_DECISION_READY_TO_PUBLISH
        );
        assert_eq!(decision.request_id, publication_plan.request_id);
        assert_eq!(decision.plan_id, publication_plan.plan_id);
        assert_eq!(
            decision.decision_id,
            final_chain_external_evm_commit_decision_id(
                publication_plan.request_id,
                publication_plan.plan_id,
                publication_plan.period,
                publication_plan.block_hash,
            )
        );
        assert_eq!(decision.publication_block_hash, publication_plan.block_hash);
        assert!(decision.error_code.is_empty());
        assert_eq!(
            session.status,
            FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_STORAGE_PUBLICATION
        );
        let step = final_chain_execution_session_next(&mut session);
        assert_eq!(
            step.action,
            FINAL_CHAIN_EXECUTION_ACTION_PUBLISH_EXTERNAL_EVM_STORAGE
        );
        assert_eq!(
            step.status,
            FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_STORAGE_PUBLICATION
        );
        assert!(step.error_code.is_empty());
    }

    #[test]
    fn external_evm_non_committed_lifecycle_after_intent_cannot_publish() {
        for (status, expected) in [
            (
                FINAL_CHAIN_EVM_LIFECYCLE_STATUS_DISCARDED,
                "FINAL_CHAIN_EVM_LIFECYCLE_DISCARDED",
            ),
            (
                FINAL_CHAIN_EVM_LIFECYCLE_STATUS_REJECTED,
                "FINAL_CHAIN_EVM_LIFECYCLE_REJECTED",
            ),
        ] {
            let (mut session, commit_plan, publication_plan) = external_evm_state_commit_session();
            let intent = final_chain_execution_session_request_external_evm_state_commit(
                &mut session,
                state_commit_request(&commit_plan, &publication_plan),
            );
            assert_eq!(
                intent.status,
                FINAL_CHAIN_EVM_STATE_COMMIT_INTENT_READY_TO_COMMIT
            );

            let decision = final_chain_execution_session_report_external_evm_lifecycle(
                &mut session,
                lifecycle_report(&commit_plan, &publication_plan, status, String::new()),
            );

            assert_eq!(decision.status, FINAL_CHAIN_EVM_COMMIT_DECISION_REJECTED);
            assert_eq!(decision.error_code, expected);
            let step = final_chain_execution_session_next(&mut session);
            assert_eq!(step.action, FINAL_CHAIN_EXECUTION_ACTION_REJECT);
            assert_eq!(step.status, FINAL_CHAIN_EXECUTION_STATUS_REJECTED);
        }
    }

    #[test]
    fn external_evm_failed_lifecycle_preserves_executor_error() {
        let (mut session, commit_plan, publication_plan) = external_evm_state_commit_session();
        let intent = final_chain_execution_session_request_external_evm_state_commit(
            &mut session,
            state_commit_request(&commit_plan, &publication_plan),
        );
        assert_eq!(
            intent.status,
            FINAL_CHAIN_EVM_STATE_COMMIT_INTENT_READY_TO_COMMIT
        );

        let decision = final_chain_execution_session_report_external_evm_lifecycle(
            &mut session,
            lifecycle_report(
                &commit_plan,
                &publication_plan,
                FINAL_CHAIN_EVM_LIFECYCLE_STATUS_REJECTED,
                "STATE_API_COMMIT_FAILED: boom".to_string(),
            ),
        );

        assert_eq!(decision.status, FINAL_CHAIN_EVM_COMMIT_DECISION_REJECTED);
        assert_eq!(
            decision.error_code,
            "FINAL_CHAIN_EVM_LIFECYCLE_REJECTED: STATE_API_COMMIT_FAILED: boom"
        );
        assert_eq!(session.status, FINAL_CHAIN_EXECUTION_STATUS_REJECTED);
    }

    #[test]
    fn external_evm_lifecycle_after_intent_must_keep_commit_facts() {
        let (mut session, commit_plan, publication_plan) = external_evm_state_commit_session();
        let intent = final_chain_execution_session_request_external_evm_state_commit(
            &mut session,
            state_commit_request(&commit_plan, &publication_plan),
        );
        assert_eq!(
            intent.status,
            FINAL_CHAIN_EVM_STATE_COMMIT_INTENT_READY_TO_COMMIT
        );
        let mut report = lifecycle_report(
            &commit_plan,
            &publication_plan,
            FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED,
            String::new(),
        );
        report.post_rewards_state_root[0] ^= 0xff;

        let decision =
            final_chain_execution_session_report_external_evm_lifecycle(&mut session, report);

        assert_eq!(decision.status, FINAL_CHAIN_EVM_COMMIT_DECISION_REJECTED);
        assert_eq!(
            decision.error_code,
            "FINAL_CHAIN_EVM_LIFECYCLE_POST_REWARDS_ROOT_MISMATCH"
        );
        assert_eq!(session.status, FINAL_CHAIN_EXECUTION_STATUS_REJECTED);
    }

    #[test]
    fn external_evm_lifecycle_rejects_wrong_committed_descriptor() {
        let (mut session, commit_plan, publication_plan) = external_evm_state_commit_session();
        let intent = final_chain_execution_session_request_external_evm_state_commit(
            &mut session,
            state_commit_request(&commit_plan, &publication_plan),
        );
        assert_eq!(
            intent.status,
            FINAL_CHAIN_EVM_STATE_COMMIT_INTENT_READY_TO_COMMIT
        );
        let mut report = lifecycle_report(
            &commit_plan,
            &publication_plan,
            FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED,
            String::new(),
        );
        report.committed_state.as_mut().unwrap().state_root[0] ^= 0xff;

        let decision =
            final_chain_execution_session_report_external_evm_lifecycle(&mut session, report);

        assert_eq!(decision.status, FINAL_CHAIN_EVM_COMMIT_DECISION_REJECTED);
        assert_eq!(
            decision.error_code,
            "FINAL_CHAIN_EVM_LIFECYCLE_COMMITTED_DESCRIPTOR_MISMATCH"
        );
    }

    fn recovery_fact() -> FinalChainExternalEvmRecoveryFact {
        let prior_state = test_prior_state();
        let request_id = [0x11; 32];
        let plan_id = [0x22; 32];
        let period = FinalChainBlockNumber::new(7);
        let publication_block_hash = [0x33; 32];
        let post_transaction_state_root = [0x44; 32];
        let post_rewards_state_root = [0x55; 32];
        let identity = FinalChainConcreteIdentity {
            policy_version: 1,
            database_id: [0x66; 32],
            chain_id: [0x77; 32],
        };
        let concrete_marker = FinalChainConcreteExecutionMarker {
            identity,
            generation: 7,
            plan_hash: [0x88; 32],
            period: period.as_u64(),
            prior_state: FinalChainConcreteState {
                period: prior_state.period.as_u64(),
                root: prior_state.state_root,
            },
            transactions_hash: [0x99; 32],
            rewards_hash: [0xaa; 32],
        };
        let expected_concrete_marker_rlp = encode_concrete_execution_marker(&concrete_marker);
        let expected_concrete_provenance_rlp =
            encode_concrete_state_provenance(&FinalChainConcreteStateProvenance {
                identity,
                generation: concrete_marker.generation,
                plan_hash: concrete_marker.plan_hash,
                committed_state: FinalChainConcreteState {
                    period: period.as_u64(),
                    root: post_rewards_state_root,
                },
                transactions_hash: concrete_marker.transactions_hash,
                rewards_hash: concrete_marker.rewards_hash,
                projection_hash: [0xbb; 32],
                catalog_hash: [0xcc; 32],
            });
        FinalChainExternalEvmRecoveryFact {
            lifecycle_id: final_chain_external_evm_lifecycle_id(
                request_id,
                plan_id,
                period,
                publication_block_hash,
                prior_state,
                post_transaction_state_root,
                post_rewards_state_root,
            ),
            request_id,
            plan_id,
            period,
            publication_block_hash,
            prior_state,
            post_transaction_state_root,
            post_rewards_state_root,
            finalized_head: prior_state,
            finalized_block_hash: None,
            finalized_block_state: None,
            committed_state: Some(FinalChainExternalEvmCommittedStateDescriptor {
                period,
                state_root: post_rewards_state_root,
            }),
            expected_concrete_marker_rlp,
            observed_concrete_provenance_rlp: expected_concrete_provenance_rlp.clone(),
            expected_concrete_provenance_rlp,
            pending_concrete_marker_rlp: Vec::new(),
        }
    }

    fn orphaned_recovery_state() -> (
        FinalChainExternalEvmCommittedStateDescriptor,
        FinalChainConcreteStateProvenance,
        FinalChainConcreteExecutionMarker,
    ) {
        let fact = recovery_fact();
        let marker = decode_concrete_execution_marker(&fact.expected_concrete_marker_rlp).unwrap();
        let committed = FinalChainConcreteState {
            period: fact.prior_state.period.as_u64(),
            root: fact.prior_state.state_root,
        };
        let provenance = FinalChainConcreteStateProvenance {
            identity: marker.identity,
            generation: marker.generation - 1,
            plan_hash: [0x42; 32],
            committed_state: committed,
            transactions_hash: [0x43; 32],
            rewards_hash: [0x44; 32],
            projection_hash: [0x45; 32],
            catalog_hash: [0x46; 32],
        };
        (fact.prior_state, provenance, marker)
    }

    #[test]
    fn orphaned_concrete_stage_authorizes_only_exact_next_generation_discard() {
        let (prior, provenance, marker) = orphaned_recovery_state();
        let encoded = encode_concrete_execution_marker(&marker);
        let discard = orphaned_concrete_discard_request(prior, &provenance, &encoded).unwrap();
        assert_eq!(discard.period, marker.period.into());
        assert_eq!(discard.prior_state, prior);
        assert_eq!(discard.concrete_marker_rlp, encoded);
        assert_eq!(discard.marker_hash, concrete_state_bytes_digest(&encoded));
    }

    #[test]
    fn orphaned_concrete_stage_rejects_foreign_ahead_and_stale_markers() {
        let (prior, provenance, marker) = orphaned_recovery_state();
        let mut cases = Vec::new();

        let mut foreign = marker.clone();
        foreign.identity.database_id[0] ^= 0xff;
        cases.push((
            foreign,
            "FINAL_CHAIN_CONCRETE_RECOVERY_ORPHAN_IDENTITY_MISMATCH",
        ));

        let mut stale = marker.clone();
        stale.generation -= 1;
        cases.push((
            stale,
            "FINAL_CHAIN_CONCRETE_RECOVERY_ORPHAN_GENERATION_MISMATCH",
        ));

        let mut wrong_prior = marker.clone();
        wrong_prior.prior_state.root[0] ^= 0xff;
        cases.push((
            wrong_prior,
            "FINAL_CHAIN_CONCRETE_RECOVERY_ORPHAN_PRIOR_MISMATCH",
        ));

        let mut ahead = marker;
        ahead.period += 1;
        cases.push((ahead, "FINAL_CHAIN_CONCRETE_RECOVERY_ORPHAN_MARKER_INVALID"));

        for (candidate, expected) in cases {
            let error = orphaned_concrete_discard_request(
                prior,
                &provenance,
                &encode_concrete_execution_marker(&candidate),
            )
            .unwrap_err();
            assert!(error.to_string().contains(expected), "{error:#}");
        }
    }

    #[test]
    fn external_evm_recovery_decides_each_crash_boundary_without_mutation() {
        let after_state_commit = recovery_fact();
        assert_eq!(
            validate_external_evm_recovery_fact(&after_state_commit).status,
            FINAL_CHAIN_EVM_RECOVERY_DECISION_READY_TO_PUBLISH
        );

        let mut before_state_commit = recovery_fact();
        before_state_commit.committed_state = Some(before_state_commit.prior_state);
        let expected =
            decode_concrete_state_provenance(&before_state_commit.expected_concrete_provenance_rlp)
                .unwrap();
        before_state_commit.observed_concrete_provenance_rlp =
            encode_concrete_state_provenance(&FinalChainConcreteStateProvenance {
                generation: expected.generation - 1,
                committed_state: FinalChainConcreteState {
                    period: before_state_commit.prior_state.period.as_u64(),
                    root: before_state_commit.prior_state.state_root,
                },
                ..expected
            });
        before_state_commit.pending_concrete_marker_rlp =
            before_state_commit.expected_concrete_marker_rlp.clone();
        assert_eq!(
            validate_external_evm_recovery_fact(&before_state_commit).status,
            FINAL_CHAIN_EVM_RECOVERY_DECISION_CLEAR_UNCOMMITTED
        );
        let before_decision = validate_external_evm_recovery_fact(&before_state_commit);
        let discard =
            external_evm_recovery_discard_request(&before_state_commit, &before_decision).unwrap();
        assert_eq!(discard.request_id, before_state_commit.request_id);
        assert_eq!(discard.period, before_state_commit.period);
        assert_eq!(discard.prior_state, before_state_commit.prior_state);
        assert_eq!(
            discard.concrete_marker_rlp,
            before_state_commit.expected_concrete_marker_rlp
        );
        assert_eq!(
            discard.marker_hash,
            concrete_state_bytes_digest(&discard.concrete_marker_rlp)
        );

        let mut after_publication = recovery_fact();
        after_publication.finalized_head = FinalChainExternalEvmCommittedStateDescriptor {
            period: after_publication.period,
            state_root: after_publication.post_rewards_state_root,
        };
        after_publication.finalized_block_hash = Some(after_publication.publication_block_hash);
        after_publication.finalized_block_state = Some(after_publication.finalized_head);
        assert_eq!(
            validate_external_evm_recovery_fact(&after_publication).status,
            FINAL_CHAIN_EVM_RECOVERY_DECISION_ALREADY_PUBLISHED
        );
    }

    #[test]
    fn external_evm_recovery_fails_closed_on_gap_ahead_root_stale_and_ambiguous_facts() {
        let mut cases = Vec::new();

        let mut gap = recovery_fact();
        gap.period = 8.into();
        cases.push((gap, "FINAL_CHAIN_EVM_RECOVERY_PERIOD_GAP"));

        let mut ahead = recovery_fact();
        ahead.committed_state = Some(FinalChainExternalEvmCommittedStateDescriptor {
            period: 8.into(),
            state_root: [0x66; 32],
        });
        cases.push((ahead, "FINAL_CHAIN_EVM_RECOVERY_COMMITTED_STATE_AHEAD"));

        let mut wrong_root = recovery_fact();
        wrong_root.committed_state.as_mut().unwrap().state_root[0] ^= 0xff;
        cases.push((
            wrong_root,
            "FINAL_CHAIN_EVM_RECOVERY_COMMITTED_ROOT_MISMATCH",
        ));

        let mut stale = recovery_fact();
        stale.finalized_head.period = stale.period;
        stale.finalized_head.state_root = stale.post_rewards_state_root;
        cases.push((stale, "FINAL_CHAIN_EVM_RECOVERY_STALE_MARKER_OR_HEAD_AHEAD"));

        let mut ambiguous = recovery_fact();
        ambiguous.committed_state = None;
        cases.push((
            ambiguous,
            "FINAL_CHAIN_EVM_RECOVERY_COMMITTED_DESCRIPTOR_MISSING",
        ));

        let mut wrong_identity = recovery_fact();
        wrong_identity.lifecycle_id[0] ^= 0xff;
        cases.push((
            wrong_identity,
            "FINAL_CHAIN_EVM_RECOVERY_LIFECYCLE_ID_MISMATCH",
        ));

        for (fact, error_code) in cases {
            let decision = validate_external_evm_recovery_fact(&fact);
            assert_eq!(decision.status, FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED);
            assert_eq!(decision.error_code, error_code);
        }
    }

    #[test]
    fn external_evm_recovery_rejects_duplicate_block_mismatch() {
        let mut fact = recovery_fact();
        fact.finalized_head = FinalChainExternalEvmCommittedStateDescriptor {
            period: fact.period,
            state_root: fact.post_rewards_state_root,
        };
        fact.finalized_block_hash = Some([0xee; 32]);
        fact.finalized_block_state = Some(fact.finalized_head);
        let decision = validate_external_evm_recovery_fact(&fact);
        assert_eq!(decision.status, FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED);
        assert_eq!(
            decision.error_code,
            "FINAL_CHAIN_EVM_RECOVERY_EXISTING_BLOCK_HASH_MISMATCH"
        );
    }

    #[test]
    fn external_evm_recovery_requires_identical_committed_provenance() {
        let mut fact = recovery_fact();
        let mut observed =
            decode_concrete_state_provenance(&fact.observed_concrete_provenance_rlp).unwrap();
        observed.projection_hash[0] ^= 0xff;
        fact.observed_concrete_provenance_rlp = encode_concrete_state_provenance(&observed);

        let decision = validate_external_evm_recovery_fact(&fact);
        assert_eq!(decision.status, FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED);
        assert_eq!(
            decision.error_code,
            "FINAL_CHAIN_EVM_RECOVERY_COMMITTED_PROVENANCE_MISMATCH"
        );
    }

    #[test]
    fn external_evm_recovery_rejects_foreign_pending_concrete_marker() {
        let mut fact = recovery_fact();
        fact.pending_concrete_marker_rlp = fact.expected_concrete_marker_rlp.clone();
        fact.pending_concrete_marker_rlp[0] ^= 0x01;

        let decision = validate_external_evm_recovery_fact(&fact);
        assert_eq!(decision.status, FINAL_CHAIN_EVM_RECOVERY_DECISION_REJECTED);
        assert_eq!(
            decision.error_code,
            "FINAL_CHAIN_EVM_RECOVERY_PENDING_CONCRETE_MARKER_MISMATCH"
        );
    }

    #[test]
    fn evm_report_rejects_bad_cumulative_gas() {
        let transactions = vec![transaction(2, Some([8; 20]), vec![0xaa])];
        let mut session = create_final_chain_execution_session(valid_request(
            transactions,
            FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
        ));
        let step = provide_system_transactions(&mut session, Vec::new());
        let tx = step.evm_request.transactions[0].clone();
        let report = FinalChainEvmExecutionReport {
            request_id: step.evm_request.request_id,
            status: FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
            prior_state: step.evm_request.prior_state,
            post_transaction_state_root: [0x11; 32],
            cumulative_gas_used: 2.into(),
            results: vec![evm_result(&tx, 1, 1, 2, vec![0xc0])],
            ..concrete_evm_report_identity(&step.evm_request)
        };

        let rejected = final_chain_execution_session_report_evm(&mut session, report);

        assert_eq!(rejected.status, FINAL_CHAIN_EXECUTION_STATUS_REJECTED);
        assert_eq!(
            rejected.error_code,
            "FINAL_CHAIN_EVM_REPORT_CUMULATIVE_GAS_MISMATCH"
        );
    }

    #[test]
    fn evm_report_rejects_invalid_transaction_status() {
        let transactions = vec![transaction(2, Some([8; 20]), vec![0xaa])];
        let mut session = create_final_chain_execution_session(valid_request(
            transactions,
            FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
        ));
        let step = provide_system_transactions(&mut session, Vec::new());
        let tx = step.evm_request.transactions[0].clone();
        let report = FinalChainEvmExecutionReport {
            request_id: step.evm_request.request_id,
            status: FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
            prior_state: step.evm_request.prior_state,
            post_transaction_state_root: [0x11; 32],
            cumulative_gas_used: 1.into(),
            results: vec![evm_result(&tx, 2, 1, 1, vec![0xc0])],
            ..concrete_evm_report_identity(&step.evm_request)
        };

        let rejected = final_chain_execution_session_report_evm(&mut session, report);

        assert_eq!(rejected.status, FINAL_CHAIN_EXECUTION_STATUS_REJECTED);
        assert_eq!(
            rejected.error_code,
            "FINAL_CHAIN_EVM_REPORT_TRANSACTION_STATUS_INVALID"
        );
    }
}
