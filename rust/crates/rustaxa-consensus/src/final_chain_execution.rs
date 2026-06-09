use crate::final_chain::{DPOS_CONTRACT_ADDRESS, FinalChain, SLASHING_CONTRACT_ADDRESS};
use crate::rewards_stats::RewardCertVoteFact;
use anyhow::Context;
use ethereum_types::{H256, U256};
use keccak_hasher::KeccakHasher;
use rustaxa_types::codec::rlp::final_chain::{
    LegacyBlockHeaderRlp, LegacyBlockHeaderRlpInput, StoredBlockHeaderRlp,
    StoredBlockHeaderRlpOwned,
};
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::{
    FinalizationDagBlock, FinalizationTransaction, LegacyTransactionEnvelope,
    StoredFinalChainBlockHeader,
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
/// Session is waiting for bridge-provided system transaction facts.
pub const FINAL_CHAIN_EXECUTION_STATUS_WAITING_SYSTEM_TRANSACTIONS: u8 = 6;
/// Session has built external-EVM commit facts and is ready to derive a
/// publication plan, but live storage commit remains disabled.
pub const FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_PUBLICATION: u8 = 7;
/// Session has derived publication facts and is waiting for external EVM state
/// lifecycle confirmation.
pub const FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_LIFECYCLE: u8 = 8;

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

/// External EVM publication was applied to Rust FinalChain storage.
pub const FINAL_CHAIN_EVM_PUBLICATION_STATUS_APPLIED: u8 = 0;
/// External EVM publication was rejected before mutating storage.
pub const FINAL_CHAIN_EVM_PUBLICATION_STATUS_REJECTED: u8 = 1;
/// External EVM publication was already present with matching block indexes.
pub const FINAL_CHAIN_EVM_PUBLICATION_STATUS_ALREADY_APPLIED: u8 = 2;

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
    pub block_gas_limit: u64,
    pub mode: u8,
}

/// One transaction in the ordered block execution stream.
///
/// When any arbitrary EVM transaction is present, the runtime exposes every
/// bridge-provided finalized transaction in block order, including native value
/// transfers and Rust-native contract actions. The executor must return
/// matching positions and hashes for the full ordered request; mismatches are
/// treated as report forgery or stale work. System transactions are still a
/// separate boundary because their creation depends on bridge-contract state
/// queries that have not moved to Rust yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalChainEvmTransactionInput {
    pub position: u64,
    pub hash: [u8; 32],
    pub sender: [u8; 20],
    pub receiver: Option<[u8; 20]>,
    pub nonce: u64,
    pub value: Vec<u8>,
    pub gas_price: Vec<u8>,
    pub gas_limit: u64,
    pub data: Vec<u8>,
    pub rlp: Vec<u8>,
    pub kind: u8,
    pub is_system: bool,
}

/// Request for system transactions needed before external EVM execution.
///
/// Rust emits this request when arbitrary EVM work is present. The temporary
/// bridge owner may answer with zero or more canonical system transaction RLPs;
/// Rust decodes them as `SystemTransaction`, appends them after regular period
/// transactions, and includes them in the external EVM request identity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainSystemTransactionRequest {
    pub request_id: [u8; 32],
    pub period: u64,
    pub regular_transaction_count: u64,
}

/// System transaction facts supplied by the bridge boundary.
///
/// The report is side-effect-free. Rust validates identity and period, decodes
/// every payload using the fixed Taraxa system sender, and rejects malformed
/// bytes before constructing the external EVM request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainSystemTransactionReport {
    pub request_id: [u8; 32],
    pub period: u64,
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
    pub period: u64,
    pub block_author: [u8; 20],
    pub timestamp: u64,
    pub block_gas_limit: u64,
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
    pub position: u64,
    pub hash: [u8; 32],
    pub status: u8,
    pub gas_used: u64,
    pub cumulative_gas_used: u64,
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
    pub cumulative_gas_used: u64,
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
    pub period: u64,
    pub block_author: [u8; 20],
    pub block_gas_used: u64,
    pub transaction_gas_used: Vec<u64>,
    pub transaction_fees: Vec<Vec<u8>>,
    pub finalized_dag_block_count: u64,
}

/// Rewards/state-root facts returned by the external EVM executor boundary.
///
/// `state_root` is the post-rewards root that will eventually enter the
/// FinalChain block header. `total_reward` is the legacy total-reward header
/// field encoded as an unsigned big-endian integer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalChainEvmRewardsReport {
    pub request_id: [u8; 32],
    pub period: u64,
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
    pub period: u64,
    pub post_execution_state_root: [u8; 32],
    pub state_root: [u8; 32],
    pub total_reward: Vec<u8>,
    pub transactions_root: [u8; 32],
    pub receipts_root: [u8; 32],
    pub header_log_bloom: Vec<u8>,
    pub indexed_log_bloom: Vec<u8>,
    pub receipts_rlp: Vec<u8>,
    pub encoded_receipts: Vec<Vec<u8>>,
    pub gas_used: u64,
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
    pub position: u32,
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
    pub current_period: u64,
    pub cache_current_period: bool,
    pub clear_cached_stats: bool,
    pub current_block_stats_rlp: Vec<u8>,
}

/// Non-mutating publication plan for an external-EVM FinalChain block.
///
/// The plan materializes the stored-header RLP, legacy full header RLP, block
/// hash, receipt payloads, transaction index facts, system-transaction hash
/// list, and rewards-stat cache mutation that Rust publishes in one storage
/// batch after a safe external EVM state lifecycle report. `plan_id` is a
/// deterministic hash of those publication facts and must be echoed by the
/// lifecycle report so stale staged-state decisions cannot be replayed across
/// blocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmPublicationPlan {
    pub request_id: [u8; 32],
    pub plan_id: [u8; 32],
    pub period: u64,
    pub block_hash: [u8; 32],
    pub block_header_rlp: Vec<u8>,
    pub stored_header_rlp: Vec<u8>,
    pub receipts_rlp: Vec<u8>,
    pub indexed_log_bloom: Vec<u8>,
    pub system_transaction_hashes_rlp: Vec<u8>,
    pub transaction_publications: Vec<FinalChainExternalEvmTransactionPublication>,
    pub executed_dag_blocks: u64,
    pub executed_transactions: u64,
    pub rewards_stats_update: FinalChainExternalEvmRewardsStatsUpdate,
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
    pub period: u64,
    pub post_execution_state_root: [u8; 32],
    pub post_rewards_state_root: [u8; 32],
    pub publication_block_hash: [u8; 32],
    pub status: u8,
    pub error_code: String,
}

/// Final Rust decision after external EVM lifecycle validation.
///
/// A ready decision proves Rust consensus has validated every deterministic
/// fact needed for external-EVM FinalChain publication. The function returning
/// this type still does not write storage; storage publication remains an
/// explicit follow-up boundary until parity and safe `StateAPI` lifecycle tests
/// are available.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmCommitDecision {
    pub request_id: [u8; 32],
    pub plan_id: [u8; 32],
    pub period: u64,
    pub publication_block_hash: [u8; 32],
    pub status: u8,
    pub error_code: String,
}

/// Result of applying an external-EVM publication plan to FinalChain storage.
///
/// The report is returned only by the explicit publication API. It does not
/// execute EVM or interact with `StateAPI`; it records whether the Rust-owned
/// FinalChain batch applied, was already present, or was rejected before any
/// storage mutation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainExternalEvmPublicationReport {
    pub request_id: [u8; 32],
    pub plan_id: [u8; 32],
    pub period: u64,
    pub block_hash: [u8; 32],
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
    pub period: u64,
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
    pub period: u64,
    pub block_header_rlp: Vec<u8>,
    pub receipts: Vec<Vec<u8>>,
    pub gas_used: u64,
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
    evm_request: Option<FinalChainEvmExecutionRequest>,
    status: u8,
    system_transaction_request: Option<FinalChainSystemTransactionRequest>,
    system_transactions: Vec<FinalChainEvmTransactionInput>,
    report: Option<FinalChainEvmExecutionReport>,
    rewards_request: Option<FinalChainEvmRewardsRequest>,
    external_evm_commit_plan: Option<FinalChainExternalEvmCommitPlan>,
    external_evm_publication_plan: Option<FinalChainExternalEvmPublicationPlan>,
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
                period: 0,
                timestamp: 0,
                extra_data: Vec::new(),
            },
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
                period: session.metadata.period,
                error_code: session.error_code.clone(),
                ..Default::default()
            }
        }
        FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM => FinalChainExecutionStep {
            status: session.status,
            action: FINAL_CHAIN_EXECUTION_ACTION_EXECUTE_EXTERNAL_EVM,
            period: session.metadata.period,
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
            period: session.metadata.period,
            external_evm_transaction_count: count_external_evm_transactions(
                &classify_ordered_execution_transactions(&session.request.transactions),
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
            period: session.metadata.period,
            evm_rewards_request: session.rewards_request.clone().unwrap_or_default(),
            error_code: session.error_code.clone(),
            ..Default::default()
        },
        FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_PUBLICATION => FinalChainExecutionStep {
            status: session.status,
            action: FINAL_CHAIN_EXECUTION_ACTION_PLAN_EXTERNAL_EVM_PUBLICATION,
            period: session.metadata.period,
            error_code: session.error_code.clone(),
            ..Default::default()
        },
        FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_LIFECYCLE => FinalChainExecutionStep {
            status: session.status,
            action: FINAL_CHAIN_EXECUTION_ACTION_REPORT_EXTERNAL_EVM_LIFECYCLE,
            period: session.metadata.period,
            error_code: session.error_code.clone(),
            ..Default::default()
        },
        FINAL_CHAIN_EXECUTION_STATUS_COMPLETE => FinalChainExecutionStep {
            status: FINAL_CHAIN_EXECUTION_STATUS_COMPLETE,
            action: FINAL_CHAIN_EXECUTION_ACTION_COMPLETE,
            period: session.metadata.period,
            ..Default::default()
        },
        _ => {
            if let Some(system_request) = session.system_transaction_request.clone() {
                session.status = FINAL_CHAIN_EXECUTION_STATUS_WAITING_SYSTEM_TRANSACTIONS;
                let external_evm_transaction_count = count_external_evm_transactions(
                    &classify_ordered_execution_transactions(&session.request.transactions),
                );
                FinalChainExecutionStep {
                    status: session.status,
                    action: FINAL_CHAIN_EXECUTION_ACTION_PROVIDE_SYSTEM_TRANSACTIONS,
                    period: session.metadata.period,
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
                    period: session.metadata.period,
                    external_evm_transaction_count,
                    evm_request,
                    error_code: String::new(),
                    ..Default::default()
                }
            } else {
                FinalChainExecutionStep {
                    status: FINAL_CHAIN_EXECUTION_STATUS_READY,
                    action: FINAL_CHAIN_EXECUTION_ACTION_COMMIT_NATIVE,
                    period: session.metadata.period,
                    external_evm_transaction_count: 0,
                    evm_request: FinalChainEvmExecutionRequest::default(),
                    error_code: String::new(),
                    ..Default::default()
                }
            }
        }
    }
}

/// Validates bridge-provided system transaction facts and constructs the
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
    if report.period != session.metadata.period {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_SYSTEM_TRANSACTIONS_PERIOD_MISMATCH".to_string();
        return final_chain_execution_session_next(session);
    }

    let regular_transactions =
        classify_ordered_execution_transactions(&session.request.transactions);
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
            &session.metadata,
            session.request.block_gas_limit,
            &all_transactions,
        ),
        period: session.metadata.period,
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
    let mut cumulative_gas_used = 0u64;
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
    let rewards_request =
        match build_external_evm_rewards_request(&session.request, request, &report) {
            Ok(request) => request,
            Err(error) => {
                session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
                session.error_code = format!("FINAL_CHAIN_EVM_REWARDS_REQUEST_INVALID: {error:#}");
                return final_chain_execution_session_next(session);
            }
        };
    session.report = Some(report);
    session.rewards_request = Some(rewards_request);
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
        return rejected_external_evm_commit_plan(&session.metadata, session.error_code.clone());
    }
    let Some(evm_request) = session.evm_request.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_WITHOUT_REQUEST".to_string();
        return rejected_external_evm_commit_plan(&session.metadata, session.error_code.clone());
    };
    let Some(evm_report) = session.report.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_WITHOUT_EVM_REPORT".to_string();
        return rejected_external_evm_commit_plan(&session.metadata, session.error_code.clone());
    };
    if rewards_report.request_id != evm_request.request_id {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_REQUEST_ID_MISMATCH".to_string();
        return rejected_external_evm_commit_plan(&session.metadata, session.error_code.clone());
    }
    if rewards_report.period != session.metadata.period {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_PERIOD_MISMATCH".to_string();
        return rejected_external_evm_commit_plan(&session.metadata, session.error_code.clone());
    }
    if rewards_report.status != FINAL_CHAIN_EVM_REWARDS_REPORT_STATUS_SUCCESS {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_REJECTED".to_string();
        return rejected_external_evm_commit_plan(&session.metadata, session.error_code.clone());
    }
    if rewards_report.total_reward.len() > 32 {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_REPORT_TOTAL_REWARD_OVERSIZED".to_string();
        return rejected_external_evm_commit_plan(&session.metadata, session.error_code.clone());
    }
    match build_external_evm_commit_plan(
        &session.request,
        &session.metadata,
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
            rejected_external_evm_commit_plan(&session.metadata, session.error_code.clone())
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
            &session.metadata,
            session.error_code.clone(),
        );
    }
    let Some(evm_request) = session.evm_request.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_PUBLICATION_WITHOUT_REQUEST".to_string();
        return rejected_external_evm_publication_plan(
            &session.metadata,
            session.error_code.clone(),
        );
    };
    let Some(commit_plan) = session.external_evm_commit_plan.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_PUBLICATION_WITHOUT_COMMIT_PLAN".to_string();
        return rejected_external_evm_publication_plan(
            &session.metadata,
            session.error_code.clone(),
        );
    };
    match build_external_evm_publication_plan(
        final_chain,
        &session.request.pbft_block_rlp,
        &session.metadata,
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
            rejected_external_evm_publication_plan(&session.metadata, session.error_code.clone())
        }
    }
}

/// Validates external EVM staged-state lifecycle facts and returns the final
/// Rust commit decision.
///
/// A committed lifecycle report must match the exact EVM execution root,
/// post-rewards root, and publication block hash derived by Rust. Ready
/// decisions remain non-mutating: the session is left rejected with an explicit
/// explicit-publication requirement so compatibility finalization cannot publish
/// external-EVM storage without calling the separate Rust publication API.
pub fn final_chain_execution_session_report_external_evm_lifecycle(
    session: &mut FinalChainExecutionSession,
    report: FinalChainExternalEvmLifecycleReport,
) -> FinalChainExternalEvmCommitDecision {
    if session.status != FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_LIFECYCLE {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_UNEXPECTED".to_string();
        return rejected_external_evm_commit_decision(
            &session.metadata,
            session.error_code.clone(),
        );
    }
    let Some(commit_plan) = session.external_evm_commit_plan.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_WITHOUT_COMMIT_PLAN".to_string();
        return rejected_external_evm_commit_decision(
            &session.metadata,
            session.error_code.clone(),
        );
    };
    let Some(publication_plan) = session.external_evm_publication_plan.as_ref() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_WITHOUT_PUBLICATION_PLAN".to_string();
        return rejected_external_evm_commit_decision(
            &session.metadata,
            session.error_code.clone(),
        );
    };
    if report.request_id != commit_plan.request_id {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_REQUEST_ID_MISMATCH".to_string();
        return rejected_external_evm_commit_decision(
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if report.plan_id != publication_plan.plan_id {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_PLAN_ID_MISMATCH".to_string();
        return rejected_external_evm_commit_decision(
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if report.period != session.metadata.period {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_PERIOD_MISMATCH".to_string();
        return rejected_external_evm_commit_decision(
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if report.post_execution_state_root != commit_plan.post_execution_state_root {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_POST_EXECUTION_ROOT_MISMATCH".to_string();
        return rejected_external_evm_commit_decision(
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if report.post_rewards_state_root != commit_plan.state_root {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_POST_REWARDS_ROOT_MISMATCH".to_string();
        return rejected_external_evm_commit_decision(
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if report.publication_block_hash != publication_plan.block_hash {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_BLOCK_HASH_MISMATCH".to_string();
        return rejected_external_evm_commit_decision(
            &session.metadata,
            session.error_code.clone(),
        );
    }
    let has_error = !report.error_code.is_empty();
    if report.status == FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED && has_error {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_LIFECYCLE_COMMITTED_WITH_ERROR".to_string();
        return rejected_external_evm_commit_decision(
            &session.metadata,
            session.error_code.clone(),
        );
    }
    if report.status != FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = if report.error_code.is_empty() {
            "FINAL_CHAIN_EVM_LIFECYCLE_NOT_COMMITTED".to_string()
        } else {
            format!(
                "FINAL_CHAIN_EVM_LIFECYCLE_NOT_COMMITTED: {}",
                report.error_code
            )
        };
        return rejected_external_evm_commit_decision(
            &session.metadata,
            session.error_code.clone(),
        );
    }

    session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
    session.error_code = "FINAL_CHAIN_EVM_PUBLICATION_REQUIRES_EXPLICIT_API".to_string();
    FinalChainExternalEvmCommitDecision {
        request_id: publication_plan.request_id,
        plan_id: publication_plan.plan_id,
        period: publication_plan.period,
        publication_block_hash: publication_plan.block_hash,
        status: FINAL_CHAIN_EVM_COMMIT_DECISION_READY_TO_PUBLISH,
        error_code: String::new(),
    }
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
            &session.metadata,
            session.error_code.clone(),
        );
    }
    let Some(publication_plan) = session.external_evm_publication_plan.take() else {
        session.status = FINAL_CHAIN_EXECUTION_STATUS_REJECTED;
        session.error_code = "FINAL_CHAIN_EVM_REWARDS_STATS_WITHOUT_PUBLICATION_PLAN".to_string();
        return rejected_external_evm_publication_plan(
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
            period: session.metadata.period,
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
    let (block_header_rlp, receipts) = final_chain.finalize_block_with_rewards_facts(
        pbft_block_rlp,
        transactions,
        finalized_dag_blocks,
        blocks_per_year,
        cert_votes,
    )?;
    Ok(FinalChainExecutionCommitReport {
        status: FINAL_CHAIN_EXECUTION_STATUS_COMPLETE,
        period: session.metadata.period,
        block_header_rlp,
        receipts,
        gas_used: 0,
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
        let ordered_transactions = classify_ordered_execution_transactions(&request.transactions);
        let external_evm_transaction_count = count_external_evm_transactions(&ordered_transactions);
        if external_evm_transaction_count == 0 {
            return Self {
                request,
                metadata,
                evm_request: None,
                status: FINAL_CHAIN_EXECUTION_STATUS_READY,
                system_transaction_request: None,
                system_transactions: Vec::new(),
                report: None,
                rewards_request: None,
                external_evm_commit_plan: None,
                external_evm_publication_plan: None,
                error_code: String::new(),
            };
        }
        if request.mode != FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED {
            return Self::rejected(
                request,
                metadata,
                "FINAL_CHAIN_EXECUTION_REQUIRES_EXTERNAL_EVM".to_string(),
            );
        }
        let system_transaction_request = FinalChainSystemTransactionRequest {
            request_id: system_transaction_request_id(
                &metadata,
                request.block_gas_limit,
                &ordered_transactions,
            ),
            period: metadata.period,
            regular_transaction_count: ordered_transactions.len() as u64,
        };
        Self {
            request,
            metadata,
            evm_request: None,
            status: FINAL_CHAIN_EXECUTION_STATUS_READY,
            system_transaction_request: Some(system_transaction_request),
            system_transactions: Vec::new(),
            report: None,
            rewards_request: None,
            external_evm_commit_plan: None,
            external_evm_publication_plan: None,
            error_code: String::new(),
        }
    }

    fn rejected(
        request: FinalChainExecutionRequest,
        metadata: rustaxa_types::PbftBlockMetadata,
        error_code: String,
    ) -> Self {
        Self {
            request,
            metadata,
            evm_request: None,
            status: FINAL_CHAIN_EXECUTION_STATUS_REJECTED,
            system_transaction_request: None,
            system_transactions: Vec::new(),
            report: None,
            rewards_request: None,
            external_evm_commit_plan: None,
            external_evm_publication_plan: None,
            error_code,
        }
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
                .ok_or_else(|| anyhow::anyhow!("system transaction position overflow"))?;
            Ok(FinalChainEvmTransactionInput {
                position: position as u64,
                hash: envelope.hash.into(),
                sender: sender.into(),
                receiver: envelope.receiver.map(Into::into),
                nonce: u256_to_u64(envelope.nonce, "system transaction nonce")?,
                value: u256_to_big_endian(envelope.value),
                gas_price: u256_to_big_endian(envelope.gas_price),
                gas_limit: envelope.gas,
                data: envelope.data,
                rlp: envelope.rlp,
                kind: FINAL_CHAIN_EXECUTION_TX_KIND_SYSTEM,
                is_system: true,
            })
        })
        .collect()
}

fn build_external_evm_rewards_request(
    finalization_request: &FinalChainExecutionRequest,
    request: &FinalChainEvmExecutionRequest,
    report: &FinalChainEvmExecutionReport,
) -> Result<FinalChainEvmRewardsRequest, anyhow::Error> {
    let mut transaction_gas_used = Vec::with_capacity(report.results.len());
    let mut transaction_fees = Vec::with_capacity(report.results.len());
    for (transaction, result) in request.transactions.iter().zip(report.results.iter()) {
        let fee = u256_from_big_endian(&transaction.gas_price)
            .checked_mul(U256::from(result.gas_used))
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
    })
}

fn build_external_evm_commit_plan(
    request: &FinalChainExecutionRequest,
    metadata: &rustaxa_types::PbftBlockMetadata,
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
    let mut indexed_log_bloom = header_log_bloom.clone();
    add_bloom_value(&mut indexed_log_bloom, metadata.author.as_bytes());
    Ok(FinalChainExternalEvmCommitPlan {
        request_id: evm_request.request_id,
        period: metadata.period,
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
    metadata: &rustaxa_types::PbftBlockMetadata,
    evm_request: &FinalChainEvmExecutionRequest,
    commit_plan: &FinalChainExternalEvmCommitPlan,
) -> Result<FinalChainExternalEvmPublicationPlan, anyhow::Error> {
    if commit_plan.error_code.is_empty() && commit_plan.request_id != evm_request.request_id {
        anyhow::bail!("external EVM publication request id mismatch");
    }
    if commit_plan.period != metadata.period {
        anyhow::bail!("external EVM publication period mismatch");
    }
    if commit_plan.header_log_bloom.len() != 256 || commit_plan.indexed_log_bloom.len() != 256 {
        anyhow::bail!("external EVM publication bloom must be 256 bytes");
    }
    if commit_plan.encoded_receipts.len() != evm_request.transactions.len() {
        anyhow::bail!(
            "external EVM publication has {} receipts for {} transaction(s)",
            commit_plan.encoded_receipts.len(),
            evm_request.transactions.len()
        );
    }

    let parent_hash = final_chain
        .block_hash(final_chain.last_block_number()?)?
        .map(|bytes| h256_from_slice(&bytes, "external EVM parent final-chain hash"))
        .transpose()?
        .unwrap_or_default();
    let stored_header = StoredFinalChainBlockHeader {
        parent_hash,
        state_root: H256::from(commit_plan.state_root),
        transactions_root: H256::from(commit_plan.transactions_root),
        receipts_root: H256::from(commit_plan.receipts_root),
        log_bloom: commit_plan.header_log_bloom.clone(),
        gas_used: commit_plan.gas_used,
        total_reward: u256_from_big_endian(&commit_plan.total_reward),
    };
    let stored_header_rlp = StoredBlockHeaderRlpOwned::from(&stored_header);
    let full_header = LegacyBlockHeaderRlp::try_from(
        LegacyBlockHeaderRlpInput::new(
            StoredBlockHeaderRlp::new(stored_header_rlp.as_bytes()),
            final_chain.block_gas_limit(),
            final_chain.genesis_timestamp(),
        )
        .signed_pbft_block(SignedPbftBlockRlp::new(pbft_block_rlp)),
    )?;
    let block_hash = full_header.hash()?;
    let transaction_publications = evm_request
        .transactions
        .iter()
        .zip(commit_plan.encoded_receipts.iter())
        .map(|(transaction, receipt)| {
            let position = u32::try_from(transaction.position)
                .map_err(|_| anyhow::anyhow!("external EVM transaction position overflow"))?;
            Ok(FinalChainExternalEvmTransactionPublication {
                transaction_hash: transaction.hash,
                position,
                is_system: transaction.is_system,
                receipt_rlp: receipt.clone(),
            })
        })
        .collect::<Result<Vec<_>, anyhow::Error>>()?;
    let mut publication = FinalChainExternalEvmPublicationPlan {
        request_id: commit_plan.request_id,
        plan_id: [0; 32],
        period: metadata.period,
        block_hash: block_hash.into(),
        block_header_rlp: full_header.into_vec(),
        stored_header_rlp: stored_header_rlp.into_vec(),
        receipts_rlp: commit_plan.receipts_rlp.clone(),
        indexed_log_bloom: commit_plan.indexed_log_bloom.clone(),
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
        rewards_stats_update: FinalChainExternalEvmRewardsStatsUpdate::default(),
        error_code: String::new(),
    };
    publication.plan_id = final_chain_external_evm_publication_plan_id(&publication);
    Ok(publication)
}

fn rejected_external_evm_commit_plan(
    metadata: &rustaxa_types::PbftBlockMetadata,
    error_code: String,
) -> FinalChainExternalEvmCommitPlan {
    FinalChainExternalEvmCommitPlan {
        period: metadata.period,
        error_code,
        ..Default::default()
    }
}

fn rejected_external_evm_publication_plan(
    metadata: &rustaxa_types::PbftBlockMetadata,
    error_code: String,
) -> FinalChainExternalEvmPublicationPlan {
    FinalChainExternalEvmPublicationPlan {
        period: metadata.period,
        error_code,
        ..Default::default()
    }
}

fn rejected_external_evm_commit_decision(
    metadata: &rustaxa_types::PbftBlockMetadata,
    error_code: String,
) -> FinalChainExternalEvmCommitDecision {
    FinalChainExternalEvmCommitDecision {
        period: metadata.period,
        status: FINAL_CHAIN_EVM_COMMIT_DECISION_REJECTED,
        error_code,
        ..Default::default()
    }
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
    stream.append(&result.gas_used);
    stream.append(&result.cumulative_gas_used);
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

fn block_log_bloom<'a>(logs: impl Iterator<Item = &'a FinalChainEvmLog>) -> Vec<u8> {
    let mut bloom = vec![0u8; 256];
    for log in logs {
        add_bloom_value(&mut bloom, &log.address);
        for topic in &log.topics {
            add_bloom_value(&mut bloom, &topic.topic);
        }
    }
    bloom
}

fn add_bloom_value(bloom: &mut [u8], value: &[u8]) {
    use tiny_keccak::{Hasher, Keccak};

    let mut hash = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(value);
    hasher.finalize(&mut hash);

    for offset in [0usize, 2, 4] {
        let bit = (((hash[offset] as usize) << 8) | hash[offset + 1] as usize) & 2047;
        let byte_index = bloom.len() - 1 - (bit / 8);
        bloom[byte_index] |= 1u8 << (bit % 8);
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

fn u256_to_u64(value: U256, field: &str) -> Result<u64, anyhow::Error> {
    if value > U256::from(u64::MAX) {
        anyhow::bail!("{field} exceeds u64");
    }
    Ok(value.low_u64())
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
    hasher.update(&plan.period.to_be_bytes());
    hasher.update(&plan.block_hash);
    hash_bytes_with_len(&mut hasher, &plan.block_header_rlp);
    hash_bytes_with_len(&mut hasher, &plan.stored_header_rlp);
    hash_bytes_with_len(&mut hasher, &plan.receipts_rlp);
    hash_bytes_with_len(&mut hasher, &plan.indexed_log_bloom);
    hash_bytes_with_len(&mut hasher, &plan.system_transaction_hashes_rlp);
    hasher.update(&(plan.transaction_publications.len() as u64).to_be_bytes());
    for publication in &plan.transaction_publications {
        hasher.update(&publication.transaction_hash);
        hasher.update(&publication.position.to_be_bytes());
        hasher.update(&[u8::from(publication.is_system)]);
        hash_bytes_with_len(&mut hasher, &publication.receipt_rlp);
    }
    hasher.update(&plan.executed_dag_blocks.to_be_bytes());
    hasher.update(&plan.executed_transactions.to_be_bytes());
    hasher.update(&plan.rewards_stats_update.current_period.to_be_bytes());
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

fn hash_bytes_with_len(hasher: &mut impl tiny_keccak::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn classify_ordered_execution_transactions(
    transactions: &[FinalizationTransaction],
) -> Vec<FinalChainEvmTransactionInput> {
    transactions
        .iter()
        .enumerate()
        .map(|(position, transaction)| {
            let kind = transaction_kind(transaction);
            FinalChainEvmTransactionInput {
                position: position as u64,
                hash: transaction.hash,
                sender: transaction.sender,
                receiver: transaction.receiver,
                nonce: transaction.nonce,
                value: transaction.value.clone(),
                gas_price: transaction.gas_price.clone(),
                gas_limit: transaction.gas_limit,
                data: transaction.data.clone(),
                rlp: transaction.rlp.clone(),
                kind,
                is_system: false,
            }
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
    metadata: &rustaxa_types::PbftBlockMetadata,
    block_gas_limit: u64,
    transactions: &[FinalChainEvmTransactionInput],
) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};

    let mut hasher = Keccak::v256();
    hasher.update(b"rustaxa-final-chain-system-transactions");
    hasher.update(&metadata.period.to_be_bytes());
    hasher.update(metadata.author.as_bytes());
    hasher.update(&metadata.timestamp.to_be_bytes());
    hasher.update(&block_gas_limit.to_be_bytes());
    for transaction in transactions {
        hasher.update(&transaction.position.to_be_bytes());
        hasher.update(&transaction.hash);
        hasher.update(&[transaction.kind]);
    }
    let mut request_id = [0u8; 32];
    hasher.finalize(&mut request_id);
    request_id
}

fn execution_request_id(
    metadata: &rustaxa_types::PbftBlockMetadata,
    block_gas_limit: u64,
    transactions: &[FinalChainEvmTransactionInput],
) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};

    let mut hasher = Keccak::v256();
    hasher.update(&metadata.period.to_be_bytes());
    hasher.update(metadata.author.as_bytes());
    hasher.update(&metadata.timestamp.to_be_bytes());
    hasher.update(&block_gas_limit.to_be_bytes());
    for transaction in transactions {
        hasher.update(&transaction.position.to_be_bytes());
        hasher.update(&transaction.hash);
        hasher.update(&transaction.sender);
        match transaction.receiver {
            Some(receiver) => {
                hasher.update(&[1]);
                hasher.update(&receiver);
            }
            None => hasher.update(&[0]),
        }
        hasher.update(&transaction.nonce.to_be_bytes());
        hasher.update(&transaction.value);
        hasher.update(&transaction.gas_price);
        hasher.update(&transaction.gas_limit.to_be_bytes());
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
            block_gas_limit: 1_000_000,
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
            nonce: 0,
            value: vec![0],
            gas_price: vec![0],
            gas_limit: 21_000,
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
            gas_used,
            cumulative_gas_used,
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
        let transactions = vec![
            transaction(1, Some([9; 20]), Vec::new()),
            transaction(2, Some(DPOS_CONTRACT_ADDRESS), vec![1, 2, 3]),
            transaction(3, Some(SLASHING_CONTRACT_ADDRESS), vec![4, 5, 6]),
        ];
        let mut session = create_final_chain_execution_session(valid_request(
            transactions,
            FINAL_CHAIN_EXECUTION_MODE_NATIVE_ONLY,
        ));

        let step = final_chain_execution_session_next(&mut session);

        assert_eq!(step.status, FINAL_CHAIN_EXECUTION_STATUS_READY);
        assert_eq!(step.action, FINAL_CHAIN_EXECUTION_ACTION_COMMIT_NATIVE);
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
        assert_eq!(system_step.system_transaction_request.period, 7);
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
        assert_eq!(step.period, 7);
        assert_eq!(step.evm_request.block_gas_limit, 1_000_000);
        assert_eq!(step.external_evm_transaction_count, 2);
        assert_eq!(step.evm_request.transactions.len(), 4);
        assert_eq!(step.evm_request.transactions[0].position, 0);
        assert_eq!(
            step.evm_request.transactions[0].kind,
            FINAL_CHAIN_EXECUTION_TX_KIND_NATIVE_VALUE_TRANSFER
        );
        assert_eq!(step.evm_request.transactions[1].position, 1);
        assert_eq!(
            step.evm_request.transactions[1].kind,
            FINAL_CHAIN_EXECUTION_TX_KIND_EXTERNAL_EVM_CALL
        );
        assert_eq!(step.evm_request.transactions[2].position, 2);
        assert_eq!(
            step.evm_request.transactions[2].kind,
            FINAL_CHAIN_EXECUTION_TX_KIND_EXTERNAL_EVM_CREATE
        );
        assert_eq!(step.evm_request.transactions[3].position, 3);
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
            cumulative_gas_used: 1,
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
            cumulative_gas_used: 1,
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
            cumulative_gas_used: 1,
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
        assert_eq!(rewards.evm_rewards_request.period, 7);
        assert_eq!(rewards.evm_rewards_request.block_gas_used, 1);
        assert_eq!(rewards.evm_rewards_request.transaction_gas_used, vec![1]);
        assert_eq!(rewards.evm_rewards_request.transaction_fees, vec![vec![0]]);
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
                cumulative_gas_used: 5,
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
                period: 7,
                status: FINAL_CHAIN_EVM_REWARDS_REPORT_STATUS_SUCCESS,
                state_root: [0x22; 32],
                total_reward: vec![0x33],
            },
        );

        assert!(plan.error_code.is_empty());
        assert_eq!(plan.period, 7);
        assert_eq!(plan.request_id, step.evm_request.request_id);
        assert_eq!(plan.post_execution_state_root, [0x10; 32]);
        assert_eq!(plan.state_root, [0x22; 32]);
        assert_eq!(plan.total_reward, vec![0x33]);
        assert_eq!(plan.gas_used, 5);
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
        assert_eq!(plan.header_log_bloom.len(), 256);
        assert_eq!(plan.indexed_log_bloom.len(), 256);
        assert!(!plan.header_log_bloom.iter().all(|byte| *byte == 0));
        assert_eq!(
            session.status,
            FINAL_CHAIN_EXECUTION_STATUS_WAITING_EXTERNAL_EVM_PUBLICATION
        );
        assert!(session.error_code.is_empty());
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
            cumulative_gas_used: 2,
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
            cumulative_gas_used: 1,
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
