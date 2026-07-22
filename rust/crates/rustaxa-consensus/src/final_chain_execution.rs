use crate::final_chain::{
    DPOS_CONTRACT_ADDRESS, FinalChain, SLASHING_CONTRACT_ADDRESS,
    external_evm_pending_publication_marker,
};
use crate::rewards_stats::{
    FinalizedRewardsPeriodFact, RewardCertVoteFact, RewardDagBlockFact, RewardTransactionFact,
    RewardsStatsPeriodRlp,
};
use anyhow::Context;
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

/// Native-only execution mode used by the current C++ `FinalChain::finalize`
/// shim while arbitrary EVM execution remains outside Rust FinalChain.
pub const FINAL_CHAIN_EXECUTION_MODE_NATIVE_ONLY: u8 = 0;
/// Mode reserved for the future C++/Rust EVM executor port. Phase 1 can build
/// and validate full ordered EVM requests and reports, but successful
/// EVM-backed commit is still rejected until system transactions, state roots,
/// rewards, and receipt publication have parity coverage.
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
/// `request_id` is deterministic for this request and must be echoed by the
/// executor report. Phase 1 does not provide a state-trie handle yet; it exposes
/// PBFT period, author, timestamp, gas limit, and the complete ordered
/// bridge-provided transaction stream needed by the future executor bridge.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainEvmExecutionRequest {
    pub request_id: [u8; 32],
    pub period: FinalChainBlockNumber,
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
/// The current runtime validates identity, ordering, cumulative gas, and basic
/// receipt RLP shape. Receipt and state-root data are retained for the future
/// commit path and deliberately do not alter storage until EVM parity is wired.
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
    pub code_error: String,
    pub consensus_error: String,
}

/// External EVM execution report returned to a runtime session.
///
/// Reports are validated against the exact request emitted by the session.
/// Successful reports are still rejected for commit until EVM state-root and
/// receipt parity are wired and covered by differential tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalChainEvmExecutionReport {
    pub request_id: [u8; 32],
    pub status: u8,
    pub state_root: [u8; 32],
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
    pub state_root: [u8; 32],
    pub total_reward: Vec<u8>,
}

/// Non-mutating Rust plan for a future external-EVM FinalChain commit.
///
/// The plan proves Rust can derive the header/storage publication facts from
/// typed EVM and rewards reports without touching `StateAPI`, `state_db/`, or
/// FinalChain storage. Production commit remains disabled until this plan is
/// connected to differential parity tests and an executor lifecycle that can
/// safely commit or discard staged EVM state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmCommitPlan {
    pub request_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub post_execution_state_root: [u8; 32],
    pub state_root: [u8; 32],
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
/// system-transaction marker, and canonical receipt RLP. Future storage commit
/// wiring can pass these facts directly to the Rust storage batch without
/// reparsing EVM receipts in C++.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmTransactionPublication {
    pub transaction_hash: [u8; 32],
    pub position: FinalChainTransactionPosition,
    pub is_system: bool,
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
    pub post_execution_state_root: [u8; 32],
    pub post_rewards_state_root: [u8; 32],
    pub publication_block_hash: [u8; 32],
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
    pub status: u8,
    pub error_code: String,
}

/// External EVM state-commit result reported by the executor boundary.
///
/// This is the narrow production boundary after Rust has already accepted a
/// [`FinalChainExternalEvmStateCommitIntent`]. C++ or a future executor adapter
/// reports only whether its staged-state commit was committed, explicitly
/// discarded, or rejected/ambiguous, plus an optional diagnostic. Rust derives
/// request id, plan id, roots, and publication block hash from the session so
/// deterministic lifecycle facts are not rematerialized outside consensus.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmStateCommitResult {
    pub status: u8,
    pub error_code: String,
}

/// External EVM staged-state lifecycle report.
///
/// The external executor owns `StateAPI`, `state_db/`, staged commit/discard,
/// and any reward-state mutation. Rust validates only stable identity facts:
/// request id, period, post-execution root, post-rewards root, publication
/// block hash, lifecycle status, and error code. It never receives EVM handles
/// or state diffs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmLifecycleReport {
    pub request_id: [u8; 32],
    pub plan_id: [u8; 32],
    pub period: FinalChainBlockNumber,
    pub post_execution_state_root: [u8; 32],
    pub post_rewards_state_root: [u8; 32],
    pub publication_block_hash: [u8; 32],
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
/// The session classifies the transaction set once, exposes either a native
/// commit step or an external EVM request, validates reports for that request,
/// and commits only native-supported execution in Phase 1.
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
/// Native-supported requests produce a `COMMIT_NATIVE` action. Requests with
/// arbitrary EVM transactions produce `EXECUTE_EXTERNAL_EVM` only when the
/// caller opted into `FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED`;
/// otherwise they are rejected explicitly.
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
            &all_transactions,
        ),
        period: session.block_number,
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
/// The returned plan contains the header roots, blooms, receipt payloads, gas,
/// and execution counters that a future storage commit path will publish in one
/// Rust-owned batch. The function intentionally does not call `StateAPI`, write
/// `DbStorage`, or mark the session complete.
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
    if rewards_report.total_reward.len() > 32 {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_TOTAL_REWARD_OVERSIZED".to_string();
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
    let prepared_rewards_stats_plan = session_external_evm_rewards_stats_plan(session)?;
    let rewards_stats_update = final_chain
        .validate_external_evm_rewards_stats_plan(prepared_rewards_stats_plan)
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
        post_execution_state_root: commit_plan.post_execution_state_root,
        post_rewards_state_root: commit_plan.state_root,
        publication_block_hash: publication_plan.block_hash,
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
        post_execution_state_root: report.post_execution_state_root,
        post_rewards_state_root: report.post_rewards_state_root,
        publication_block_hash: report.publication_block_hash,
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
/// The caller supplies only the executor boundary outcome. Rust reconstructs
/// the full lifecycle report from the session-owned state-commit intent and
/// commit plan, validates it through the same lifecycle path as compatibility
/// tests, and advances to storage publication only for committed outcomes. An
/// explicit discarded outcome clears the pending publication marker because the
/// external owner has confirmed staged state did not commit. Rejected outcomes
/// keep the marker intact so startup recovery can arbitrate ambiguous
/// post-commit failures using the durable `StateAPI` descriptor.
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
    let decision = final_chain_execution_session_report_external_evm_lifecycle(
        session,
        FinalChainExternalEvmLifecycleReport {
            request_id: intent.request_id,
            plan_id: intent.plan_id,
            period: intent.period,
            post_execution_state_root: commit_plan.post_execution_state_root,
            post_rewards_state_root: commit_plan.state_root,
            publication_block_hash: intent.publication_block_hash,
            status,
            error_code: result.error_code,
        },
    );
    if status == FINAL_CHAIN_EVM_LIFECYCLE_STATUS_DISCARDED
        && decision.status == FINAL_CHAIN_EVM_COMMIT_DECISION_REJECTED
    {
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
            commit_plan.post_execution_state_root,
            commit_plan.state_root,
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
/// Only sessions whose next step is `COMMIT_NATIVE` are allowed to publish
/// FinalChain storage. External EVM sessions must stay rejected until a real
/// executor report commit path is implemented and parity-tested.
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
        if external_evm_transaction_count == 0 {
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
        post_execution_state_root: evm_report.state_root,
        state_root: rewards_report.state_root,
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
        state_root: H256::from(commit_plan.state_root),
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
    if request.post_execution_state_root != commit_plan.post_execution_state_root {
        return Err(format!("{error_prefix}_POST_EXECUTION_ROOT_MISMATCH"));
    }
    if request.post_rewards_state_root != commit_plan.state_root {
        return Err(format!("{error_prefix}_POST_REWARDS_ROOT_MISMATCH"));
    }
    if request.publication_block_hash != publication_plan.block_hash {
        return Err(format!("{error_prefix}_BLOCK_HASH_MISMATCH"));
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

fn u256_to_nonce_bytes(value: U256) -> Vec<u8> {
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
    hasher.update(b"rustaxa-final-chain-external-evm-publication-plan-v1");
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

fn hash_bytes_with_len(hasher: &mut impl tiny_keccak::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
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
    transactions: &[FinalChainEvmTransactionInput],
) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};

    let mut hasher = Keccak::v256();
    // Version the identity domain because nonce encoding changed from fixed
    // eight-byte integers to length-prefixed canonical arbitrary-width bytes.
    hasher.update(b"rustaxa-final-chain-evm-execution-v2");
    hasher.update(&block_number.as_u64().to_be_bytes());
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

    fn provide_system_transactions(
        session: &mut FinalChainExecutionSession,
        transactions: Vec<Vec<u8>>,
    ) -> FinalChainExecutionStep {
        let request = final_chain_execution_session_next(session);
        assert_eq!(
            request.action,
            FINAL_CHAIN_EXECUTION_ACTION_PROVIDE_SYSTEM_TRANSACTIONS
        );
        final_chain_execution_session_report_system_transactions(
            session,
            FinalChainSystemTransactionReport {
                request_id: request.system_transaction_request.request_id,
                period: request.period,
                transactions,
            },
        )
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
        let commit_plan = FinalChainExternalEvmCommitPlan {
            request_id: [0x11; 32],
            period: 7.into(),
            post_execution_state_root: [0x22; 32],
            state_root: [0x33; 32],
            error_code: String::new(),
            ..Default::default()
        };
        let publication_plan = FinalChainExternalEvmPublicationPlan {
            request_id: commit_plan.request_id,
            plan_id: [0x44; 32],
            period: 7.into(),
            block_hash: [0x55; 32],
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
                18, 236, 103, 98, 108, 95, 176, 9, 221, 61, 118, 64, 74, 213, 156, 213, 236, 177,
                126, 165, 0, 183, 163, 157, 189, 136, 62, 110, 114, 218, 93, 207,
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
            post_execution_state_root: commit_plan.post_execution_state_root,
            post_rewards_state_root: commit_plan.state_root,
            publication_block_hash: publication_plan.block_hash,
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
            post_execution_state_root: commit_plan.post_execution_state_root,
            post_rewards_state_root: commit_plan.state_root,
            publication_block_hash: publication_plan.block_hash,
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
            &[transaction.clone()],
        );
        assert_eq!(
            legacy_width_id,
            [
                74, 153, 100, 167, 85, 120, 13, 212, 97, 101, 51, 7, 95, 140, 95, 38, 36, 154, 52,
                176, 63, 21, 193, 30, 106, 26, 206, 102, 240, 251, 79, 35,
            ]
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
                &[fixed_width],
            )
        );
        transaction.nonce = transaction.nonce.next();
        let widened_id = execution_request_id(
            FinalChainBlockNumber::new(metadata.period),
            &metadata,
            1_000_000.into(),
            &[transaction],
        );
        assert_ne!(legacy_width_id, widened_id);
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
        assert_eq!(
            execution_request_id(
                FinalChainBlockNumber::new(metadata.period),
                &metadata,
                1_000_000.into(),
                &[transaction],
            ),
            [
                90, 76, 93, 113, 145, 192, 173, 31, 4, 216, 240, 230, 23, 235, 240, 15, 84, 116,
                68, 209, 160, 185, 34, 246, 15, 54, 235, 249, 218, 168, 223, 153,
            ]
        );
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
            state_root: [0x11; 32],
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
            state_root: [0x11; 32],
            cumulative_gas_used: 1.into(),
            results: vec![mismatched_result],
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
            state_root: [0x11; 32],
            cumulative_gas_used: 1.into(),
            results: vec![evm_result_with_encoded_receipt(&tx, 1, 1, 1)],
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
                state_root: [0x11; 32],
                cumulative_gas_used: 1.into(),
                results: vec![evm_result_with_encoded_receipt(&tx, 1, 1, 1)],
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
        let rewards = final_chain_execution_session_report_evm(
            &mut session,
            FinalChainEvmExecutionReport {
                request_id: step.evm_request.request_id,
                status: FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
                state_root: [0x10; 32],
                cumulative_gas_used: 5.into(),
                results: vec![first.clone(), second.clone()],
            },
        );
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
                state_root: [0x22; 32],
                total_reward: vec![0x33],
            },
        );

        assert!(plan.error_code.is_empty());
        assert_eq!(plan.period, 7.into());
        assert_eq!(plan.request_id, step.evm_request.request_id);
        assert_eq!(plan.post_execution_state_root, [0x10; 32]);
        assert_eq!(plan.state_root, [0x22; 32]);
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
            state_root: [0x11; 32],
            cumulative_gas_used: 2.into(),
            results: vec![evm_result(&tx, 1, 1, 2, vec![0xc0])],
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
            state_root: [0x11; 32],
            cumulative_gas_used: 1.into(),
            results: vec![evm_result(&tx, 2, 1, 1, vec![0xc0])],
        };

        let rejected = final_chain_execution_session_report_evm(&mut session, report);

        assert_eq!(rejected.status, FINAL_CHAIN_EXECUTION_STATUS_REJECTED);
        assert_eq!(
            rejected.error_code,
            "FINAL_CHAIN_EVM_REPORT_TRANSACTION_STATUS_INVALID"
        );
    }
}
