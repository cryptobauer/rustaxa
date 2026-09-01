use crate::gas_pricer::{GasPriceOracle, GasPricerConfig};
use crate::transaction_manager::{
    DagTransactionSaveFact, FinalizedTransactionFilterFact, FinalizedTransactionFilterPlan,
    FinalizedTransactionStatusFact, FinalizedTransactionStatusPlan,
    TransactionManagerInsertTransactionFact, TransactionManagerInsertTransactionStatus,
    TransactionManagerKnownFact, TransactionManagerSidecar, TransactionManagerSidecarRecoveryEntry,
    TransactionManagerValidatedInsertFact, TransactionManagerVerifyTransactionFact,
    TransactionManagerVerifyTransactionStatus, VerifyNotFinalizedTransactionFact,
    plan_exclude_finalized_transactions, plan_finalized_transactions_status,
    plan_insert_transaction, plan_transactions_from_dag_block, plan_validated_insert,
    plan_verify_not_finalized_transactions, plan_verify_transaction,
};
use crate::transaction_packing_service::{
    TransactionPackingCandidate, TransactionPackingEffect, TransactionPackingEstimate,
    TransactionPackingEstimateRequest, TransactionPackingOwner, TransactionPackingRequest,
    TransactionPackingSelection, TransactionPackingService,
};
use crate::transaction_queue::{
    TransactionQueue, TransactionQueueDemoteStatus, TransactionQueueEntry,
    TransactionQueueInsertStatus,
};
use crate::transaction_storage::{
    NonFinalizedTransactionStoragePayload, STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR,
    STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM, STORED_TRANSACTION_SOURCE_MISSING,
    STORED_TRANSACTION_SOURCE_PENDING, StoredTransactionLookupRequest,
    append_non_finalized_transactions_to_batch, load_non_finalized_recovery_entries,
    load_stored_transactions, remove_non_finalized_transactions, save_transaction_count,
    transaction_finalized,
};
use anyhow::{Context, Result, anyhow, ensure};
use ethereum_types::{H160, H256, U256};
use rlp::Rlp;
use rustaxa_storage::StorageWriteBatch;
use rustaxa_storage::{StatusField, Storage};
use rustaxa_types::LegacyTransactionEnvelope;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const TRANSACTION_QUEUE_DROP_WINDOW: Duration = Duration::from_secs(600);
const TRANSACTIONS_POS_IN_PERIOD_DATA: usize = 3;

/// Stable failure identifier returned when the native transaction lock is poisoned.
pub const DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED: &str =
    "DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED";

/// Transaction view source used by native verifier/lookup APIs.
pub const TM_TRANSACTION_VIEW_SOURCE_MISSING: u8 = 0;
/// Transaction view source for queue-backed transaction payloads.
pub const TM_TRANSACTION_VIEW_SOURCE_QUEUE: u8 = 1;
/// Transaction view source for non-finalized transaction sidecar payloads.
pub const TM_TRANSACTION_VIEW_SOURCE_NON_FINALIZED_SIDECAR: u8 = 2;
/// Transaction view source for recently-finalized transaction sidecar payloads.
pub const TM_TRANSACTION_VIEW_SOURCE_RECENTLY_FINALIZED_SIDECAR: u8 = 3;
/// Transaction view source for pending storage payloads.
pub const TM_TRANSACTION_VIEW_SOURCE_STORAGE_PENDING: u8 = 4;
/// Transaction view source for finalized regular storage payloads.
pub const TM_TRANSACTION_VIEW_SOURCE_STORAGE_FINALIZED_REGULAR: u8 = 5;
/// Transaction view source for finalized system storage payloads.
pub const TM_TRANSACTION_VIEW_SOURCE_STORAGE_FINALIZED_SYSTEM: u8 = 6;
/// No finalized transaction was found by native verification.
pub const TM_VERIFY_NOT_FINALIZED_SOURCE_NONE: u8 = 0;
/// Verification stopped on native recently-finalized sidecar state.
pub const TM_VERIFY_NOT_FINALIZED_SOURCE_RECENT_SIDECAR: u8 = 1;
/// Verification stopped on durable finalized transaction state.
pub const TM_VERIFY_NOT_FINALIZED_SOURCE_STORAGE: u8 = 2;

/// CXX-free request for a single transaction view lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionServiceTransactionViewRequest {
    /// Caller-owned input position.
    pub input_index: u64,
    /// Canonical transaction hash for the requested view.
    pub hash: [u8; 32],
}

/// CXX-free verified transaction view output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionServiceTransactionView {
    /// Caller-owned input position from the request.
    pub input_index: u64,
    /// Canonical transaction hash from the request.
    pub hash: [u8; 32],
    /// Whether a payload was found in queue, sidecar, or storage.
    pub found: bool,
    /// Source tag matching one of `TM_TRANSACTION_VIEW_SOURCE_*`.
    pub source: u8,
    /// Whether a finalized storage payload is older than a supplied sender nonce.
    pub old_finalized: bool,
    /// Canonical transaction payload. Empty when not found.
    pub tx_rlp: Vec<u8>,
}

/// Bounded transaction view lookup output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionServiceTransactionViewPlan {
    /// Number of requested items considered from the front of input.
    pub requested_count: u64,
    /// Whether lookup completed all requested views.
    pub complete: bool,
    /// Result views in input order.
    pub views: Vec<TransactionServiceTransactionView>,
}

/// CXX-free account nonce fact for finalized storage filtering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionServiceAccountNonceFact {
    /// Sender account used by the proposal transaction.
    pub sender: [u8; 20],
    /// True when the sender account exists in FinalChain state.
    pub account_found: bool,
    /// Sender finalized nonce at proposal period.
    pub account_nonce: [u8; 32],
}

/// CXX-free request for finalized transaction filtering.
///
/// Requests preserve caller order and identity through the returned actions.
/// A zero hash is rejected before its storage lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionServiceFinalizedFilterRequest {
    /// Caller-owned input position.
    pub input_index: u64,
    /// Canonical transaction identity.
    pub hash: H256,
}

/// CXX-free fact for verifying one transaction against finalized state.
///
/// Recently-finalized sidecar membership short-circuits immediately. Durable
/// storage is queried only when `sender_account_nonce >= transaction_nonce`;
/// zero hashes and storage errors fail the complete operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionServiceVerifyNotFinalizedFact {
    /// Caller-owned input position.
    pub input_index: u64,
    /// Canonical transaction identity.
    pub hash: H256,
    /// Nonce declared by the transaction.
    pub transaction_nonce: U256,
    /// Sender nonce supplied by the retained FinalChain boundary.
    pub sender_account_nonce: U256,
}

/// First finalized transaction found by native verification.
///
/// The output identifies the first finalized input in request order and its
/// native source. When every input passes, `is_finalized` is false and the
/// remaining fields use their zero/none sentinels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionServiceVerifyNotFinalizedOutcome {
    /// True when verification stopped on a finalized transaction.
    pub is_finalized: bool,
    /// Caller-owned input position, or zero when all inputs pass.
    pub input_index: u64,
    /// Finalized transaction identity, or zero when all inputs pass.
    pub hash: H256,
    /// One of the `TM_VERIFY_NOT_FINALIZED_SOURCE_*` tags.
    pub source: u8,
}

/// CXX-free facts for one validated transaction admission attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionServiceValidatedAdmissionFact {
    /// Canonical transaction identity.
    pub tx_hash: H256,
    /// Transaction nonce decoded from the canonical envelope.
    pub transaction_nonce: U256,
    /// Maximum account debit implied by the transaction.
    pub transaction_cost: U256,
    /// Transaction gas limit.
    pub gas_limit: u64,
    /// Current proposal DAG gas limit.
    pub proposal_dag_gas_limit: u64,
    /// Whether invalid latest-state transactions may remain non-proposable.
    pub insert_non_proposable: bool,
}

/// Latest FinalChain facts supplied by the retained external execution boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionServiceFinalChainAdmissionFact {
    /// Whether the sender account exists.
    pub account_found: bool,
    /// Latest sender nonce.
    pub account_nonce: U256,
    /// Latest sender balance.
    pub account_balance: U256,
    /// Finalized transaction period when the hash is already finalized.
    pub finalized_period: Option<u64>,
}

/// Native result of one transaction admission command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionServiceAdmissionReport {
    /// Public insertion status selected after queue mutation.
    pub insert_status: TransactionManagerInsertTransactionStatus,
    /// Queue mutation status.
    pub transaction_status: TransactionQueueInsertStatus,
    /// Finalized period for an already-finalized transaction.
    pub finalized_period: Option<u64>,
    /// Hash inserted into the queue, when insertion published one.
    pub inserted_hash: Option<H256>,
    /// Whether the retained shell should emit its transaction-added event.
    pub emit_transaction_added: bool,
}

/// Legacy public result text selected by native admission behavior.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionServicePublicInsertResult {
    /// Whether public insertion succeeded.
    pub accepted: bool,
    /// Stable legacy message, empty on success.
    pub message: String,
}

/// Native result of public precheck, verification, and admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionServicePublicAdmissionReport {
    /// Deterministic verification status.
    pub verification_status: TransactionManagerVerifyTransactionStatus,
    /// Transaction chain identifier.
    pub verification_chain_id: u64,
    /// Configured chain identifier.
    pub verification_expected_chain_id: u64,
    /// Public compatibility result.
    pub public_result: TransactionServicePublicInsertResult,
    /// Admission result; absent when verification rejected the transaction.
    pub admission: Option<TransactionServiceAdmissionReport>,
}

/// Canonical payload fact for finalized-status publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionServiceFinalizedStatusFact {
    /// Caller-owned position used to preserve payload/action identity.
    pub input_index: u64,
    /// Canonical transaction hash.
    pub hash: H256,
    /// Canonical transaction RLP retained in the recently-finalized sidecar.
    pub tx_rlp: Vec<u8>,
}

/// Native effects produced after finalized-status persistence and publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionServiceFinalizedStatusReport {
    /// Hashes removed from the non-finalized sidecar.
    pub removed_non_finalized: Vec<H256>,
    /// Hashes erased directly from the live queue.
    pub queue_erased: Vec<H256>,
    /// Hashes purged because their account nonce finalized.
    pub finalized_account_purged: Vec<H256>,
    /// Number of accepted finalized payloads.
    pub accepted_count: u64,
}

/// Decodes canonical finalized transaction facts from one legacy `PeriodData` payload.
///
/// The transaction list is read from the canonical period-data position and
/// each raw transaction item is inspected with the shared legacy envelope
/// decoder. Outputs preserve input order and canonical bytes while deriving
/// hashes natively. Malformed period data or transaction RLP returns an error
/// before transaction state is locked or mutated.
pub fn finalized_status_facts_from_period_data(
    period_data_rlp: &[u8],
) -> Result<Vec<TransactionServiceFinalizedStatusFact>> {
    let period_data = Rlp::new(period_data_rlp);
    let transactions = period_data
        .at(TRANSACTIONS_POS_IN_PERIOD_DATA)
        .context("TM_FINALIZED_STATUS_PERIOD_DATA_TRANSACTIONS")?;
    finalized_status_facts_from_rlp_list(transactions)
}

/// Decodes canonical finalized transaction facts from one RLP transaction list.
///
/// This supports the retained non-PBFT C++ compatibility method when its
/// caller owns only a partially populated `PeriodData` object that cannot be
/// canonically serialized. Each transaction is still decoded and hashed in
/// Rust; malformed list or transaction RLP fails before state mutation.
pub fn finalized_status_facts_from_transaction_list_rlp(
    transaction_list_rlp: &[u8],
) -> Result<Vec<TransactionServiceFinalizedStatusFact>> {
    finalized_status_facts_from_rlp_list(Rlp::new(transaction_list_rlp))
}

fn finalized_status_facts_from_rlp_list(
    transactions: Rlp<'_>,
) -> Result<Vec<TransactionServiceFinalizedStatusFact>> {
    let count = transactions
        .item_count()
        .context("TM_FINALIZED_STATUS_PERIOD_DATA_TRANSACTION_COUNT")?;
    (0..count)
        .map(|index| {
            let tx_rlp = transactions
                .at(index)
                .with_context(|| format!("TM_FINALIZED_STATUS_TRANSACTION_AT_{index}"))?
                .as_raw()
                .to_vec();
            let envelope = LegacyTransactionEnvelope::decode(&tx_rlp)
                .with_context(|| format!("TM_FINALIZED_STATUS_TRANSACTION_DECODE_{index}"))?;
            Ok(TransactionServiceFinalizedStatusFact {
                input_index: index as u64,
                hash: envelope.hash,
                tx_rlp,
            })
        })
        .collect()
}

fn public_verification_result(
    status: TransactionManagerVerifyTransactionStatus,
    chain_id: u64,
    expected_chain_id: u64,
) -> TransactionServicePublicInsertResult {
    let message = match status {
        TransactionManagerVerifyTransactionStatus::Accepted => "",
        TransactionManagerVerifyTransactionStatus::ChainIdMismatch => {
            return TransactionServicePublicInsertResult {
                accepted: false,
                message: format!("chain_id mismatch {chain_id} {expected_chain_id}"),
            };
        }
        TransactionManagerVerifyTransactionStatus::InvalidGas => "invalid gas",
        TransactionManagerVerifyTransactionStatus::IntrinsicGasNotCovered => {
            "intrinsic gas too low"
        }
        TransactionManagerVerifyTransactionStatus::InvalidSignature => "invalid signature",
        TransactionManagerVerifyTransactionStatus::GasPriceTooLow => "gas_price too low",
    };
    TransactionServicePublicInsertResult {
        accepted: status == TransactionManagerVerifyTransactionStatus::Accepted,
        message: message.to_string(),
    }
}

fn public_insert_result(
    admission: &TransactionServiceAdmissionReport,
) -> TransactionServicePublicInsertResult {
    match admission.insert_status {
        TransactionManagerInsertTransactionStatus::Accepted => {
            TransactionServicePublicInsertResult {
                accepted: true,
                message: String::new(),
            }
        }
        TransactionManagerInsertTransactionStatus::AlreadyKnown => {
            TransactionServicePublicInsertResult {
                accepted: false,
                message: "Transaction already in transactions pool".to_string(),
            }
        }
        TransactionManagerInsertTransactionStatus::AlreadyFinalized => {
            TransactionServicePublicInsertResult {
                accepted: false,
                message: format!(
                    "Transaction already finalized in period{}",
                    admission.finalized_period.unwrap_or_default()
                ),
            }
        }
        TransactionManagerInsertTransactionStatus::CouldNotInsert => {
            TransactionServicePublicInsertResult {
                accepted: false,
                message: "Transaction could not be inserted".to_string(),
            }
        }
    }
}

/// CXX-free request for one public transaction gas-estimation decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionServiceGasEstimationRequest {
    /// Canonical transaction identity.
    pub hash: H256,
    /// Gas declared by the transaction.
    pub declared_gas: u64,
    /// Proposal period that scopes cached estimation results.
    pub proposal_period: u64,
    /// Declared-gas threshold below which EVM execution is unnecessary.
    pub estimate_gas_limit: u64,
}

/// Native decision for a public gas-estimation request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionServiceGasEstimationPlan {
    /// Use the transaction's declared gas without an EVM call.
    Declared { gas_used: u64 },
    /// Return the proposal-period cache entry and its opaque execution result.
    Cached { gas_used: u64, result_rlp: Vec<u8> },
    /// Execute the retained external EVM leaf and report the result separately.
    ExecuteEvm,
}

#[derive(Clone)]
struct TransactionServiceStoredTransactionRequest {
    input_index: u64,
    hash: [u8; 32],
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct TransactionServiceStoredTransactionLookup {
    hash: [u8; 32],
    found: bool,
    source: u8,
    old_finalized: bool,
    tx_rlp: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StoredTransactionIdentity {
    sender: [u8; 20],
    nonce: U256,
}

/// Immutable inputs for constructing and restoring the native transaction owner.
///
/// Queue and gas-cache capacities are derived from `queue_max_size`. The gas
/// oracle validates its own percentile configuration and restores finalized
/// history according to `gas_pricer_config`. `proposal_dag_gas_limit` is the
/// queue-weight limit used by pool-mode gas-price bids.
#[derive(Clone, Debug)]
pub struct TransactionServiceConfig {
    /// Maximum number of transactions retained by the native queue.
    pub queue_max_size: usize,
    /// Gas-oracle policy and restoration mode.
    pub gas_pricer_config: GasPricerConfig,
    /// Proposal weight used when deriving a pool gas-price bid.
    pub proposal_dag_gas_limit: u64,
}

/// Complete mutable transaction application state.
///
/// The state is published only inside [`TransactionService`] after transaction
/// count and gas-history restoration succeed. Its public fields are an internal
/// application-composition surface; callers must hold a [`TransactionServiceGuard`]
/// and must not retain references across external executor callbacks or expose
/// them through CXX.
pub struct TransactionServiceState {
    /// Live transaction count, payload sidecars, and gas-estimation cache.
    pub sidecar: TransactionManagerSidecar,
    /// Native pending-transaction queue.
    pub queue: TransactionQueue,
    /// Finalized-history gas oracle.
    pub gas_price_oracle: GasPriceOracle,
    /// Proposal weight used to derive the queue inclusion price.
    pub proposal_dag_gas_limit: u64,
    /// Shared durable storage used by migrated transaction routes.
    pub storage: Arc<Storage>,
    /// Last queue-drop observation used by compatibility telemetry.
    pub last_drop_observed: Option<Instant>,
    /// Native owner of the current proposal-packing session.
    pub transaction_packing: TransactionPackingService,
}

/// Native fact for one transaction considered during DAG-block persistence.
#[derive(Clone)]
pub struct DagTransactionSaveInput {
    /// Stable position in the caller's canonical DAG-block transaction order.
    pub input_index: u64,
    /// Canonical transaction identity.
    pub hash: H256,
    /// Canonical encoded transaction bytes persisted for later finalization.
    pub transaction_rlp: Vec<u8>,
    /// Transaction nonce used to gate finalized-storage lookup.
    pub transaction_nonce: U256,
    /// Latest sender-account nonce supplied by the retained FinalChain boundary.
    pub sender_account_nonce: U256,
}

/// Native accepted-transaction publication result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DagTransactionSaveAccepted {
    /// Canonical transaction identity.
    pub hash: H256,
    /// Whether publication erased the transaction from the live queue.
    pub erased_from_queue: bool,
}

/// Native live-state result for a committed DAG transaction save.
#[derive(Debug, Eq, PartialEq)]
pub struct DagTransactionSaveOutcome {
    /// Accepted transactions in canonical input order.
    pub accepted: Vec<DagTransactionSaveAccepted>,
}

/// Prepared persistence retained until a shared DAG/transaction batch commits.
pub(crate) struct PreparedDagTransactionSave {
    accepted: Vec<DagTransactionSaveAccepted>,
    accepted_payloads: Vec<NonFinalizedTransactionStoragePayload>,
    target_transaction_count: u64,
}

/// Fully prevalidated transaction live-state publication.
pub(crate) struct PreparedDagTransactionPublication {
    queue: TransactionQueue,
    sidecar: TransactionManagerSidecar,
    outcome: DagTransactionSaveOutcome,
}

/// Prepared transaction owner output returned by one proposer-packing step.
#[derive(Clone, Debug)]
pub(crate) struct TransactionServiceProposerPackPrepared {
    /// Candidates that still need estimator input from the compatibility EVM boundary.
    pub request_estimates: Vec<TransactionServiceEstimateRequest>,
    /// Deterministic selections already accepted without external estimation.
    pub selected_transactions: Vec<TransactionPackingSelection>,
}

/// Executor-ready transaction candidate returned across the unlocked EVM boundary.
///
/// Construction decodes and cross-checks the canonical queue payload while the
/// transaction lock is held. Bridge conversion is therefore infallible and a
/// malformed queue entry cannot strand the paired DAG and packing cursors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionServiceEstimateRequest {
    /// Canonical transaction identity.
    pub hash: H256,
    /// Canonical signed transaction RLP for the concrete EVM leaf.
    pub transaction_rlp: Vec<u8>,
    /// Declared transaction gas used by legacy estimator routing.
    pub declared_gas: u64,
    /// Recovered sender, validated against queue metadata.
    pub sender: ethereum_types::H160,
    /// Transaction nonce.
    pub nonce: U256,
    /// Transaction gas price.
    pub gas_price: U256,
    /// Transaction gas limit.
    pub gas: u64,
    /// Optional receiver; `None` denotes contract creation.
    pub receiver: Option<ethereum_types::H160>,
    /// Transaction value.
    pub value: U256,
    /// Calldata or initcode.
    pub data: Vec<u8>,
}

/// Finalized proposer-packing output after all required estimator results are known.
#[derive(Clone, Debug)]
pub(crate) struct TransactionServiceProposerPackFinalized {
    /// Deterministic selected proposals with canonical RLP payloads.
    pub selected_transactions: Vec<TransactionPackingSelection>,
}

/// Complete native output of compatibility pack preparation.
///
/// All payloads are owned and validated before the transaction lock is
/// released. A non-empty estimate list means the native compatibility cursor
/// remains active; otherwise preparation is terminal.
#[derive(Clone, Debug)]
pub struct TransactionServiceCompatibilityPackPrepared {
    /// Compatibility-session candidates that still need estimator input.
    pub request_estimates: Vec<TransactionServiceEstimateRequest>,
    /// Deterministic selections already accepted without external estimation.
    pub selected_transactions: Vec<TransactionPackingSelection>,
    /// Hashes demoted by the runtime session outcome.
    pub demoted_hashes: Vec<H256>,
    /// Whether planner stop condition was triggered.
    pub stopped: bool,
}

/// Complete native output of compatibility pack finalization.
///
/// Selections and demotions preserve planner order. Construction succeeds only
/// after the exact owner, input count, and hash sequence are validated.
#[derive(Clone, Debug)]
pub struct TransactionServiceCompatibilityPackFinalized {
    /// Deterministic selected proposals with canonical RLP payloads.
    pub selected_transactions: Vec<TransactionPackingSelection>,
    /// Hashes demoted by the runtime session outcome.
    pub demoted_hashes: Vec<H256>,
    /// Whether planner stop condition was triggered.
    pub stopped: bool,
}

/// CXX-free inputs for one compatibility transaction-packing session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionServiceCompatibilityPackRequest {
    /// Maximum cumulative proposal gas weight.
    pub weight_limit: u64,
    /// Minimum gas charged when deriving the candidate snapshot bound.
    pub min_transaction_gas: u64,
    /// Proposal period used for cache and shard selection.
    pub proposal_period: u64,
    /// Declared-gas ceiling below which EVM execution is unnecessary.
    pub estimate_gas_limit: u64,
    /// FinalChain head recorded on queue demotions.
    pub last_block_number: u64,
    /// Number of configured transaction shards.
    pub total_shards: u16,
    /// Local transaction shard.
    pub node_shard: u16,
    /// Periods per transaction-shard rotation.
    pub shard_period_interval: u64,
}

/// CXX-free external estimate for a pending compatibility pack.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionServicePackEstimate {
    /// Candidate identity, validated against the pending cursor.
    pub hash: H256,
    /// Gas used by the retained external EVM executor.
    pub gas_used: u64,
    /// FinalChain head associated with any resulting demotion.
    pub last_block_number: u64,
    /// Opaque execution-result RLP retained by the native cache.
    pub result_rlp: Vec<u8>,
}

/// CXX-free opaque gas-estimation cache insertion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionServiceGasEstimationResult {
    /// Canonical transaction identity.
    pub hash: H256,
    /// Proposal period forming the cache key.
    pub proposal_period: u64,
    /// Estimated gas used.
    pub gas_used: u64,
    /// Opaque external execution-result RLP.
    pub result_rlp: Vec<u8>,
}

/// Owned canonical payload used by transaction-sidecar mutation tasks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionServicePayload {
    /// Canonical transaction identity.
    pub hash: H256,
    /// Canonical transaction RLP.
    pub transaction_rlp: Vec<u8>,
}

/// Native owner of transaction construction, restoration, and serialization.
///
/// One mutex protects queue, sidecar/cache/count, gas oracle, durable storage,
/// drop observation, and the packing subowner. Poisoning is mapped to the stable
/// [`DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED`] identifier.
pub struct TransactionService {
    state: Mutex<TransactionServiceState>,
}

impl TransactionService {
    /// Restores and publishes the complete transaction runtime.
    ///
    /// The durable transaction count and any enabled finalized-block gas-price
    /// history are restored before the mutex-owning service is constructed.
    /// Missing count metadata retains storage's canonical zero behavior.
    /// Light/full-node missing-history behavior is delegated unchanged to the
    /// gas oracle. Any validation, storage, or decode error publishes no owner.
    pub fn restore(storage: Arc<Storage>, config: TransactionServiceConfig) -> Result<Self> {
        Ok(Self {
            state: Mutex::new(TransactionServiceState::restore(storage, config)?),
        })
    }

    /// Locks the complete transaction serialization domain.
    ///
    /// The returned guard exposes native state only to short-lived Rust
    /// adapters. A poisoned lock returns the stable transaction-lock identifier.
    pub fn lock(&self) -> Result<TransactionServiceGuard<'_>> {
        Ok(TransactionServiceGuard(self.state.lock().map_err(
            |_| anyhow!(DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED),
        )?))
    }

    /// Returns the queue-aware gas-price bid from one native lock epoch.
    ///
    /// The returned value is an owned big-endian scalar. A poisoned transaction
    /// lock returns the stable native lock error and no state reference escapes.
    pub fn gas_price_bid(&self) -> Result<[u8; 32]> {
        Ok(self.lock()?.gas_price_bid())
    }

    /// Plans one gas-estimation request under the native transaction lock.
    ///
    /// The returned enum makes the declared, cached, and external-EVM paths
    /// mutually exclusive. The lock is released before the caller executes EVM
    /// work, and no cache reference escapes.
    pub fn plan_gas_estimation(
        &self,
        request: TransactionServiceGasEstimationRequest,
    ) -> Result<TransactionServiceGasEstimationPlan> {
        self.lock()?.plan_gas_estimation(request)
    }

    /// Returns the durable transaction count from one native lock epoch.
    pub fn transaction_count(&self) -> Result<u64> {
        Ok(self.lock()?.transaction_count())
    }

    /// Persists and publishes transactions accepted from one DAG block.
    ///
    /// Filtering, finalized-storage lookup, transaction-count authority, the
    /// durable write batch, and live queue/sidecar publication execute under
    /// one native transaction lock. Live state is prevalidated on clones and
    /// is published only after a successful storage commit. Empty or fully
    /// filtered input performs no write. Any planning, validation, storage, or
    /// lock error leaves live state unpublished.
    pub fn save_dag_transactions(
        &self,
        facts: Vec<DagTransactionSaveInput>,
    ) -> Result<DagTransactionSaveOutcome> {
        self.save_dag_transactions_with_committer(facts, |storage, batch| {
            storage.commit_write_batch_with_sync(batch, false)
        })
    }

    fn save_dag_transactions_with_committer(
        &self,
        facts: Vec<DagTransactionSaveInput>,
        commit: impl FnOnce(&Storage, StorageWriteBatch) -> Result<()>,
    ) -> Result<DagTransactionSaveOutcome> {
        let mut runtime = self.lock()?;
        let prepared = prepare_dag_transactions(&runtime, facts)?;
        let publication = prepare_dag_transaction_publication(&runtime, &prepared)?;
        if !prepared.accepted_payloads.is_empty() {
            let mut batch = runtime.storage.create_write_batch();
            append_prepared_dag_transactions(&runtime.storage, &mut batch, &prepared)?;
            commit(&runtime.storage, batch).context("TM_DAG_TX_BATCH_COMMIT")?;
        }
        Ok(publish_dag_transactions(&mut runtime, publication))
    }

    /// Returns whether the queue or sidecar knows `hash`.
    ///
    /// Queue-known and sidecar membership are evaluated while holding the same
    /// transaction lock. Sidecar validation failures are returned unchanged.
    pub fn is_transaction_known(&self, hash: [u8; 32]) -> Result<bool> {
        self.lock()?.is_transaction_known(hash)
    }

    /// Returns the current non-finalized sidecar cardinality.
    pub fn non_finalized_size(&self) -> Result<usize> {
        Ok(self.lock()?.non_finalized_size())
    }

    /// Returns queue-only transaction views in request order.
    ///
    /// Missing entries are explicit empty views. All payloads are owned before
    /// the native lock is released.
    pub fn queue_transaction_views(
        &self,
        requests: Vec<TransactionServiceTransactionViewRequest>,
    ) -> Result<Vec<TransactionServiceTransactionView>> {
        Ok(self.lock()?.queue_transaction_views(requests))
    }

    /// Returns non-finalized sidecar transaction views in request order.
    pub fn non_finalized_transaction_views(
        &self,
        requests: Vec<TransactionServiceTransactionViewRequest>,
    ) -> Result<Vec<TransactionServiceTransactionView>> {
        self.lock()?.non_finalized_transaction_views(requests)
    }

    /// Returns bounded transaction views using queue, sidecar, then storage precedence.
    ///
    /// Storage access remains inside the transaction lock so the returned owned
    /// views describe one serialization epoch.
    pub fn transaction_views(
        &self,
        requests: Vec<TransactionServiceTransactionViewRequest>,
        max_count: u64,
    ) -> Result<TransactionServiceTransactionViewPlan> {
        self.lock()?.lookup_transaction_views(requests, max_count)
    }

    /// Returns proposal-period views with permissive optional account-nonce facts.
    ///
    /// Missing nonce facts preserve the public compatibility lookup semantics;
    /// this method deliberately does not use the verifier's fact-required path.
    pub fn proposal_transaction_views(
        &self,
        proposal_period: u64,
        requests: Vec<TransactionServiceTransactionViewRequest>,
        account_nonce_facts: Vec<TransactionServiceAccountNonceFact>,
        max_count: u64,
    ) -> Result<TransactionServiceTransactionViewPlan> {
        self.lock()?
            .lookup_proposal_transaction_views_with_account_nonce_facts(
                proposal_period,
                requests,
                account_nonce_facts,
                max_count,
            )
    }

    /// Returns proposer transaction groups ordered by sender and nonce.
    pub fn queue_transaction_groups(&self) -> Result<Vec<Vec<TransactionQueueEntry>>> {
        Ok(self.lock()?.queue_transaction_groups())
    }

    /// Returns the current proposable transaction count.
    pub fn queue_size(&self) -> Result<usize> {
        Ok(self.lock()?.queue_size())
    }

    /// Returns the current ordered proposable-account set.
    pub fn queue_proposable_accounts(&self) -> Result<Vec<H160>> {
        Ok(self.lock()?.queue_proposable_accounts())
    }

    /// Returns whether the queue drop-observation window remains active.
    pub fn queue_transactions_dropped(&self) -> Result<bool> {
        Ok(self.lock()?.queue_transactions_dropped())
    }

    /// Returns whether non-proposable queue state reached its configured bound.
    pub fn queue_non_proposable_over_limit(&self) -> Result<bool> {
        Ok(self.lock()?.queue_non_proposable_over_limit())
    }

    /// Returns the minimum big-endian gas price for inclusion under `limit`.
    pub fn queue_min_gas_price_for_block_inclusion(&self, limit: u64) -> Result<[u8; 32]> {
        Ok(self.lock()?.queue_min_gas_price_for_block_inclusion(limit))
    }

    /// Updates finalized-block gas-price history in one native lock epoch.
    ///
    /// Empty input and pool-mode owners retain the gas oracle's no-op
    /// semantics. No state reference escapes the operation.
    pub fn update_gas_prices(&self, gas_prices: Vec<U256>) -> Result<()> {
        self.lock()?.update_gas_prices(gas_prices);
        Ok(())
    }

    /// Starts the compatibility transaction pack under native lock ownership.
    ///
    /// Candidate selection, cache reads, payload validation, demotions, and
    /// cursor publication are atomic with respect to transaction state. The
    /// returned owned requests cross the unlocked external-EVM interval.
    pub fn prepare_compatibility_pack(
        &self,
        request: TransactionServiceCompatibilityPackRequest,
    ) -> Result<TransactionServiceCompatibilityPackPrepared> {
        self.lock()?.prepare_proposer_pack_for_owner(
            TransactionPackingOwner::Compatibility,
            request.weight_limit,
            request.min_transaction_gas,
            request.proposal_period,
            request.estimate_gas_limit,
            request.last_block_number,
            request.total_shards,
            request.node_shard,
            request.shard_period_interval,
        )
    }

    /// Finalizes the active compatibility pack from ordered external estimates.
    ///
    /// Owner, count, and hash mismatches retain the matching cursor without
    /// publishing queue or cache state.
    pub fn finalize_compatibility_pack(
        &self,
        estimates: Vec<TransactionServicePackEstimate>,
    ) -> Result<TransactionServiceCompatibilityPackFinalized> {
        self.lock()?.finalize_proposer_pack_for_owner(
            TransactionPackingOwner::Compatibility,
            estimates
                .into_iter()
                .map(|estimate| TransactionPackingEstimate {
                    hash: estimate.hash,
                    gas_used: estimate.gas_used,
                    last_block_number: estimate.last_block_number,
                    result_rlp: estimate.result_rlp,
                })
                .collect(),
        )
    }

    /// Aborts only an active compatibility-owned packing cursor.
    pub fn abort_compatibility_pack(&self) -> Result<bool> {
        self.lock()?
            .abort_proposer_pack(TransactionPackingOwner::Compatibility)
    }

    /// Stores one opaque external EVM result in the native estimation cache.
    pub fn store_gas_estimation(
        &self,
        result: TransactionServiceGasEstimationResult,
    ) -> Result<bool> {
        self.lock()?.store_gas_estimation(result)
    }

    /// Inserts canonical payloads and moves them to recently-finalized state.
    ///
    /// All payload mutations occur while the transaction owner is locked;
    /// malformed sidecar state returns without exposing a guard.
    pub fn initialize_recently_finalized(
        &self,
        period: u64,
        payloads: Vec<TransactionServicePayload>,
    ) -> Result<()> {
        self.lock()?.initialize_recently_finalized(period, payloads)
    }

    /// Removes selected non-finalized payloads durably before live publication.
    ///
    /// Zero hashes fail before storage mutation. Storage failure leaves the
    /// native sidecar unchanged.
    pub fn remove_non_finalized(&self, hashes: Vec<H256>) -> Result<u64> {
        self.lock()?.remove_non_finalized(hashes)
    }

    /// Applies finalized-block expiry to non-proposable queue state.
    pub fn queue_block_finalized(&self, block_number: u64) -> Result<Vec<H256>> {
        Ok(self.lock()?.queue_block_finalized(block_number))
    }

    /// Filters finalized hashes using sidecar and durable state in one lock epoch.
    ///
    /// Input order and caller indices are preserved for every non-finalized
    /// action. Recent-sidecar hits precede storage checks; zero hashes or
    /// storage failures return an error without exposing a state guard.
    pub fn filter_non_finalized(
        &self,
        requests: Vec<TransactionServiceFinalizedFilterRequest>,
    ) -> Result<FinalizedTransactionFilterPlan> {
        self.lock()?.filter_non_finalized(requests)
    }

    /// Returns the first finalized transaction using supplied FinalChain nonce facts.
    ///
    /// Verification short-circuits in input order. Recent-sidecar hits do not
    /// consult storage, and durable lookup occurs only when the supplied sender
    /// nonce covers the transaction nonce. Zero hashes and storage failures
    /// return an error.
    pub fn verify_not_finalized(
        &self,
        facts: Vec<TransactionServiceVerifyNotFinalizedFact>,
    ) -> Result<TransactionServiceVerifyNotFinalizedOutcome> {
        self.lock()?.verify_not_finalized(facts)
    }

    /// Rebuilds live non-finalized sidecars from durable native storage.
    ///
    /// Stale finalized rows are removed by the storage loader before survivor
    /// envelopes are decoded, cost-checked, and hash/sender-validated. All
    /// survivors publish atomically from a cloned sidecar; validation failure
    /// preserves the prior live sidecar and returns a stable contextual error.
    pub fn recover_non_finalized(&self) -> Result<u64> {
        self.lock()?.recover_non_finalized()
    }

    /// Executes one validated admission under the native transaction lock.
    ///
    /// The caller supplies only decoded envelope and retained FinalChain facts.
    /// Rust validates carrier identity, plans queue eligibility, mutates the
    /// queue, and selects the public status and shell-event fact atomically.
    pub fn execute_admission(
        &self,
        fact: TransactionServiceValidatedAdmissionFact,
        final_chain_fact: TransactionServiceFinalChainAdmissionFact,
        entry: TransactionQueueEntry,
    ) -> Result<TransactionServiceAdmissionReport> {
        self.lock()?
            .execute_admission(fact, final_chain_fact, entry)
    }

    /// Executes public known-fast-path, verification, and admission behavior.
    ///
    /// Known state is checked before verification to preserve legacy error
    /// precedence. Verification rejection does not mutate the queue. All
    /// returned data is owned before the native transaction lock is released.
    pub fn execute_public_admission(
        &self,
        verify_fact: TransactionManagerVerifyTransactionFact,
        admission_fact: TransactionServiceValidatedAdmissionFact,
        final_chain_fact: TransactionServiceFinalChainAdmissionFact,
        entry: TransactionQueueEntry,
    ) -> Result<TransactionServicePublicAdmissionReport> {
        self.lock()?
            .execute_public_admission(verify_fact, admission_fact, final_chain_fact, entry)
    }

    /// Persists and publishes finalized transaction status plus periodic queue purge.
    ///
    /// Count persistence precedes all live publication. The complete status
    /// transition and account-nonce purge occur under one native transaction
    /// lock, and the returned owned hashes are logging/effect facts only.
    pub fn update_finalized_status(
        &self,
        period: u64,
        retention_window: u64,
        account_nonce_facts: Vec<TransactionServiceAccountNonceFact>,
        facts: Vec<TransactionServiceFinalizedStatusFact>,
    ) -> Result<TransactionServiceFinalizedStatusReport> {
        self.lock()?
            .update_finalized_status(period, retention_window, account_nonce_facts, facts)
    }

    /// Applies finalized status directly from canonical legacy `PeriodData`.
    ///
    /// Canonical transaction facts are decoded before acquiring the native
    /// transaction lock. The mutation then uses the same storage-first,
    /// sidecar, queue, account-purge, and count semantics as
    /// [`Self::update_finalized_status`].
    pub fn update_finalized_status_from_period_data(
        &self,
        period: u64,
        retention_window: u64,
        account_nonce_facts: Vec<TransactionServiceAccountNonceFact>,
        period_data_rlp: &[u8],
    ) -> Result<TransactionServiceFinalizedStatusReport> {
        let facts = finalized_status_facts_from_period_data(period_data_rlp)?;
        self.update_finalized_status(period, retention_window, account_nonce_facts, facts)
    }
}

/// Exclusive native transaction runtime guard.
///
/// The guard dereferences to the complete state and releases the transaction
/// lock on drop. It must never cross CXX or an EVM, FinalChain, network, event,
/// logging, thread-pool, or asynchronous executor boundary.
pub struct TransactionServiceGuard<'a>(MutexGuard<'a, TransactionServiceState>);

impl Deref for TransactionServiceGuard<'_> {
    type Target = TransactionServiceState;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TransactionServiceGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl TransactionServiceState {
    /// Returns the transaction-pressure facts captured by a DAG proposer cursor.
    ///
    /// Both counts are observed under the transaction service lock. The snapshot
    /// contains no queue or sidecar references and may safely outlive the guard.
    pub(crate) fn dag_proposer_transaction_pressure(&self) -> (u64, u64) {
        (self.queue.size(), self.sidecar.non_finalized_size() as u64)
    }

    /// Restores the complete state used by [`TransactionService`].
    ///
    /// This constructor is exposed for focused native mechanics tests.
    /// Production publication must use
    /// [`TransactionService::restore`] so a partially restored state cannot
    /// escape without its native lock owner.
    pub fn restore(storage: Arc<Storage>, config: TransactionServiceConfig) -> Result<Self> {
        let initial_transaction_count = storage
            .metadata()
            .status_field(StatusField::TrxCount as u8)
            .context("TM_RUNTIME_TRANSACTION_COUNT_READ")?;
        let blocks_gas_pricer = config.gas_pricer_config.blocks_gas_pricer;
        let mut gas_price_oracle = GasPriceOracle::new(config.gas_pricer_config)?;
        if blocks_gas_pricer {
            gas_price_oracle
                .restore_from_storage(storage.as_ref())
                .context("TM_RUNTIME_GAS_PRICE_HISTORY_RESTORE")?;
        }
        let gas_estimation_cache_size = config.queue_max_size / 10;
        let gas_estimation_cache_delete_step = config.queue_max_size / 100;
        Ok(Self {
            sidecar: TransactionManagerSidecar::new_with_gas_estimation_cache(
                initial_transaction_count,
                gas_estimation_cache_size,
                gas_estimation_cache_delete_step,
            ),
            queue: TransactionQueue::new(config.queue_max_size as u64),
            gas_price_oracle,
            proposal_dag_gas_limit: config.proposal_dag_gas_limit,
            storage,
            last_drop_observed: None,
            transaction_packing: TransactionPackingService::new(),
        })
    }

    /// Returns bounded, source-ordered transaction views from queue, sidecars, and storage.
    pub(crate) fn lookup_transaction_views(
        &self,
        requests: Vec<TransactionServiceTransactionViewRequest>,
        max_count: u64,
    ) -> Result<TransactionServiceTransactionViewPlan> {
        transaction_service_runtime_lookup_transaction_views_inner(
            self,
            requests,
            max_count,
            |stored_requests| {
                transaction_service_load_stored_transactions_from_storage(
                    self.storage.as_ref(),
                    stored_requests,
                )
            },
        )
    }

    /// Returns bounded, source-ordered finalized-period proposal views with optional nonce filtering.
    pub(crate) fn lookup_proposal_transaction_views_with_account_nonce_facts(
        &self,
        proposal_period: u64,
        requests: Vec<TransactionServiceTransactionViewRequest>,
        account_nonce_facts: Vec<TransactionServiceAccountNonceFact>,
        max_count: u64,
    ) -> Result<TransactionServiceTransactionViewPlan> {
        transaction_service_runtime_lookup_transaction_views_inner(
            self,
            requests,
            max_count,
            |stored_requests| {
                transaction_service_load_proposal_transactions_with_account_nonce_facts_from_storage(
                    self.storage.as_ref(),
                    proposal_period,
                    account_nonce_facts,
                    stored_requests,
                    false,
                )
            },
        )
    }

    /// Returns queue-only payload views.
    pub(crate) fn queue_transaction_views(
        &self,
        requests: Vec<TransactionServiceTransactionViewRequest>,
    ) -> Vec<TransactionServiceTransactionView> {
        requests
            .into_iter()
            .map(
                |request| match self.queue.transaction(H256::from(request.hash)) {
                    Some(entry) => TransactionServiceTransactionView {
                        input_index: request.input_index,
                        hash: request.hash,
                        found: true,
                        source: TM_TRANSACTION_VIEW_SOURCE_QUEUE,
                        old_finalized: false,
                        tx_rlp: entry.rlp,
                    },
                    None => TransactionServiceTransactionView {
                        input_index: request.input_index,
                        hash: request.hash,
                        found: false,
                        source: TM_TRANSACTION_VIEW_SOURCE_MISSING,
                        old_finalized: false,
                        tx_rlp: Vec::new(),
                    },
                },
            )
            .collect()
    }

    /// Returns deterministic hash-known state as seen by the Rust runtime.
    pub(crate) fn is_transaction_known(&self, hash: [u8; 32]) -> Result<bool> {
        let hash = H256::from(hash);
        self.sidecar
            .is_transaction_known(TransactionManagerKnownFact {
                hash,
                queue_known: self.queue.is_transaction_known(hash),
            })
    }

    fn execute_admission(
        &mut self,
        fact: TransactionServiceValidatedAdmissionFact,
        final_chain_fact: TransactionServiceFinalChainAdmissionFact,
        entry: TransactionQueueEntry,
    ) -> Result<TransactionServiceAdmissionReport> {
        ensure!(
            entry.hash == fact.tx_hash,
            "TM_RUNTIME_VALIDATED_INSERT_HASH_MISMATCH"
        );
        ensure!(
            entry.nonce == fact.transaction_nonce,
            "TM_RUNTIME_VALIDATED_INSERT_NONCE_MISMATCH"
        );
        ensure!(
            entry.gas == fact.gas_limit,
            "TM_RUNTIME_VALIDATED_INSERT_GAS_MISMATCH"
        );

        let plan = plan_validated_insert(TransactionManagerValidatedInsertFact {
            tx_hash: fact.tx_hash,
            transaction_nonce: fact.transaction_nonce,
            transaction_cost: fact.transaction_cost,
            gas_limit: fact.gas_limit,
            propose_dag_gas_limit: fact.proposal_dag_gas_limit,
            insert_non_proposable: fact.insert_non_proposable,
            in_non_finalized_cache: self.sidecar.contains_non_finalized(fact.tx_hash),
            in_recently_finalized_cache: self.sidecar.contains_recently_finalized(fact.tx_hash),
            account_found: final_chain_fact.account_found,
            account_nonce: final_chain_fact.account_nonce,
            account_balance: final_chain_fact.account_balance,
        })?;

        let (queue_status, inserted_hash) = if plan.should_insert_queue {
            let outcome = self.queue.insert(entry, plan.queue_proposable)?;
            if outcome.status == TransactionQueueInsertStatus::Overflow
                || !outcome.overflow_removed_hashes.is_empty()
            {
                self.last_drop_observed = Some(Instant::now());
            }
            (outcome.status, outcome.inserted_hash)
        } else {
            (plan.status, None)
        };
        let insert = plan_insert_transaction(TransactionManagerInsertTransactionFact {
            tx_hash: fact.tx_hash,
            hash_known: false,
            queue_status,
            has_finalized_period: final_chain_fact.finalized_period.is_some(),
            finalized_period: final_chain_fact.finalized_period.unwrap_or_default(),
        })?;

        Ok(TransactionServiceAdmissionReport {
            insert_status: insert.status,
            transaction_status: queue_status,
            finalized_period: insert.finalized_period,
            inserted_hash,
            emit_transaction_added: plan.emit_transaction_added
                && queue_status == TransactionQueueInsertStatus::Inserted,
        })
    }

    fn execute_public_admission(
        &mut self,
        verify_fact: TransactionManagerVerifyTransactionFact,
        admission_fact: TransactionServiceValidatedAdmissionFact,
        final_chain_fact: TransactionServiceFinalChainAdmissionFact,
        entry: TransactionQueueEntry,
    ) -> Result<TransactionServicePublicAdmissionReport> {
        ensure!(
            verify_fact.tx_hash == admission_fact.tx_hash,
            "TM_RUNTIME_PUBLIC_INSERT_VERIFY_HASH_MISMATCH"
        );
        let chain_id = verify_fact.chain_id;
        let expected_chain_id = verify_fact.expected_chain_id;
        let hash_known = self
            .sidecar
            .is_transaction_known(TransactionManagerKnownFact {
                hash: verify_fact.tx_hash,
                queue_known: self.queue.is_transaction_known(verify_fact.tx_hash),
            })
            .context("TM_RUNTIME_INSERT_PRECHECK_KNOWN_CHECK_FAILED")?;
        let precheck = plan_insert_transaction(TransactionManagerInsertTransactionFact {
            tx_hash: verify_fact.tx_hash,
            hash_known,
            queue_status: TransactionQueueInsertStatus::Inserted,
            has_finalized_period: false,
            finalized_period: 0,
        })?;
        if precheck.status != TransactionManagerInsertTransactionStatus::Accepted {
            let admission = TransactionServiceAdmissionReport {
                insert_status: precheck.status,
                transaction_status: if precheck.status
                    == TransactionManagerInsertTransactionStatus::AlreadyKnown
                {
                    TransactionQueueInsertStatus::Known
                } else {
                    TransactionQueueInsertStatus::Inserted
                },
                finalized_period: precheck.finalized_period,
                inserted_hash: None,
                emit_transaction_added: false,
            };
            return Ok(TransactionServicePublicAdmissionReport {
                verification_status: TransactionManagerVerifyTransactionStatus::Accepted,
                verification_chain_id: chain_id,
                verification_expected_chain_id: expected_chain_id,
                public_result: public_insert_result(&admission),
                admission: Some(admission),
            });
        }

        let verification = plan_verify_transaction(verify_fact)?;
        if verification.status != TransactionManagerVerifyTransactionStatus::Accepted {
            return Ok(TransactionServicePublicAdmissionReport {
                verification_status: verification.status,
                verification_chain_id: chain_id,
                verification_expected_chain_id: expected_chain_id,
                public_result: public_verification_result(
                    verification.status,
                    chain_id,
                    expected_chain_id,
                ),
                admission: None,
            });
        }

        let admission = self.execute_admission(admission_fact, final_chain_fact, entry)?;
        Ok(TransactionServicePublicAdmissionReport {
            verification_status: TransactionManagerVerifyTransactionStatus::Accepted,
            verification_chain_id: chain_id,
            verification_expected_chain_id: expected_chain_id,
            public_result: public_insert_result(&admission),
            admission: Some(admission),
        })
    }

    fn update_finalized_status(
        &mut self,
        period: u64,
        retention_window: u64,
        account_nonce_facts: Vec<TransactionServiceAccountNonceFact>,
        facts: Vec<TransactionServiceFinalizedStatusFact>,
    ) -> Result<TransactionServiceFinalizedStatusReport> {
        let plan: FinalizedTransactionStatusPlan = plan_finalized_transactions_status(
            facts
                .iter()
                .map(|fact| FinalizedTransactionStatusFact {
                    input_index: fact.input_index,
                    hash: fact.hash,
                    in_non_finalized_cache: self.sidecar.contains_non_finalized(fact.hash),
                })
                .collect(),
            self.sidecar.transaction_count(),
            period,
            retention_window,
        )?;
        let validated_facts = plan
            .accepted_transactions
            .iter()
            .map(|action| {
                let fact = facts
                    .get(action.input_index as usize)
                    .context("TM_RUNTIME_FINALIZED_STATUS_INPUT_INDEX")?;
                ensure!(
                    fact.hash == action.hash,
                    "TM_RUNTIME_FINALIZED_STATUS_HASH_MISMATCH"
                );
                Ok(fact)
            })
            .collect::<Result<Vec<_>>>()?;
        if !plan.accepted_transactions.is_empty() {
            save_transaction_count(self.storage.as_ref(), plan.target_transaction_count)
                .context("TM_FINALIZED_STATUS_TRXCOUNT_WRITE")?;
        }
        if let Some(stale_period) = plan.stale_period {
            self.sidecar
                .evict_recently_finalized_stale_period(stale_period);
        }

        let mut removed_non_finalized = Vec::new();
        let mut queue_erased = Vec::new();
        for (action, fact) in plan.accepted_transactions.iter().zip(validated_facts) {
            self.sidecar
                .insert_recently_finalized(period, fact.hash, fact.tx_rlp.clone())
                .context("TM_RUNTIME_FINALIZED_STATUS_INSERT")?;
            self.queue.mark_transaction_known(fact.hash);
            if self.queue.erase(fact.hash) {
                queue_erased.push(fact.hash);
            }
            if action.removed_non_finalized {
                removed_non_finalized.push(fact.hash);
            }
        }
        self.sidecar
            .set_transaction_count(plan.target_transaction_count);

        let supplied: HashMap<H160, (bool, U256)> = account_nonce_facts
            .into_iter()
            .map(|fact| {
                (
                    H160::from(fact.sender),
                    (
                        fact.account_found,
                        U256::from_big_endian(&fact.account_nonce),
                    ),
                )
            })
            .collect();
        let purge_facts = self
            .queue
            .proposable_accounts()
            .into_iter()
            .map(|sender| {
                let (account_found, account_nonce) =
                    supplied.get(&sender).copied().unwrap_or_default();
                crate::transaction_queue::TransactionQueueAccountNonceFact {
                    sender,
                    account_found,
                    account_nonce,
                }
            })
            .collect::<Vec<_>>();
        let finalized_account_purged = if plan.purge_transactions {
            self.queue.purge_accounts_plan(&purge_facts).removed_hashes
        } else {
            Vec::new()
        };

        Ok(TransactionServiceFinalizedStatusReport {
            removed_non_finalized,
            queue_erased,
            finalized_account_purged,
            accepted_count: plan.accepted_transactions.len() as u64,
        })
    }

    /// Returns proposal views that require an explicit account fact for each finalized sender.
    pub(crate) fn lookup_proposal_transaction_views_requiring_account_nonce_facts(
        &self,
        proposal_period: u64,
        requests: Vec<TransactionServiceTransactionViewRequest>,
        account_nonce_facts: Vec<TransactionServiceAccountNonceFact>,
        max_count: u64,
    ) -> Result<TransactionServiceTransactionViewPlan> {
        transaction_service_runtime_lookup_transaction_views_inner(
            self,
            requests,
            max_count,
            |stored_requests| {
                transaction_service_load_proposal_transactions_with_account_nonce_facts_from_storage(
                    self.storage.as_ref(),
                    proposal_period,
                    account_nonce_facts,
                    stored_requests,
                    true,
                )
            },
        )
    }

    /// Returns non-finalized/recently-finalized sidecar payload views.
    pub(crate) fn non_finalized_transaction_views(
        &self,
        requests: Vec<TransactionServiceTransactionViewRequest>,
    ) -> Result<Vec<TransactionServiceTransactionView>> {
        self.sidecar
            .lookup_payloads_ordered(
                requests
                    .into_iter()
                    .map(|request| (request.input_index, H256::from(request.hash)))
                    .collect(),
            )
            .context("TM_RUNTIME_TRANSACTION_VIEW_NON_FINALIZED_LOOKUP")?
            .into_iter()
            .map(|lookup| {
                let found = lookup.found
                    && lookup.source
                        == crate::transaction_manager::TransactionManagerSidecarLookup::SOURCE_NON_FINALIZED;
                Ok(TransactionServiceTransactionView {
                    input_index: lookup.input_index,
                    hash: lookup.hash.0,
                    found,
                    source: if found {
                        TM_TRANSACTION_VIEW_SOURCE_NON_FINALIZED_SIDECAR
                    } else {
                        TM_TRANSACTION_VIEW_SOURCE_MISSING
                    },
                    old_finalized: false,
                    tx_rlp: if found { lookup.trx_rlp } else { Vec::new() },
                })
            })
            .collect()
    }

    /// Returns the current transaction count.
    pub(crate) fn transaction_count(&self) -> u64 {
        self.sidecar.transaction_count()
    }

    /// Returns non-finalized sidecar size.
    pub(crate) fn non_finalized_size(&self) -> usize {
        self.sidecar.non_finalized_size()
    }

    /// Returns queue-only payload groups by sender/nonce.
    pub(crate) fn queue_transaction_groups(&self) -> Vec<Vec<TransactionQueueEntry>> {
        self.queue.all_transaction_groups()
    }

    /// Returns queue entry count currently available for proposer scans.
    pub(crate) fn queue_size(&self) -> usize {
        self.queue.size() as usize
    }

    /// Returns proposable accounts currently tracked by Rust queue.
    pub(crate) fn queue_proposable_accounts(&self) -> Vec<H160> {
        self.queue.proposable_accounts()
    }

    /// Returns whether queue overflow/drop telemetry is currently warm.
    pub(crate) fn queue_transactions_dropped(&self) -> bool {
        self.last_drop_observed
            .is_some_and(|observed| observed.elapsed() < TRANSACTION_QUEUE_DROP_WINDOW)
    }

    /// Returns whether queue size by account exceeds the configured drop bound.
    pub(crate) fn queue_non_proposable_over_limit(&self) -> bool {
        self.queue.non_proposable_transactions_over_the_limit()
    }

    /// Returns the minimum inclusion gas price estimate for a weight limit.
    pub(crate) fn queue_min_gas_price_for_block_inclusion(&self, limit: u64) -> [u8; 32] {
        self.queue
            .min_gas_price_for_block_inclusion(limit)
            .to_big_endian()
    }

    /// Returns the runtime gas bid as selected by pool/proposal context.
    pub(crate) fn gas_price_bid(&self) -> [u8; 32] {
        self.gas_price_oracle
            .configured_bid(
                self.queue
                    .min_gas_price_for_block_inclusion(self.proposal_dag_gas_limit),
            )
            .to_big_endian()
    }

    pub(crate) fn plan_gas_estimation(
        &self,
        request: TransactionServiceGasEstimationRequest,
    ) -> Result<TransactionServiceGasEstimationPlan> {
        ensure!(
            !request.hash.is_zero(),
            "TM_RUNTIME_GAS_ESTIMATION_HASH_ZERO"
        );
        if request.declared_gas <= request.estimate_gas_limit {
            return Ok(TransactionServiceGasEstimationPlan::Declared {
                gas_used: request.declared_gas,
            });
        }
        if let Some(cached) = self
            .sidecar
            .gas_estimation_cache_get(request.hash, request.proposal_period)
            .context("TM_RUNTIME_GAS_ESTIMATION_CACHE_GET")?
        {
            return Ok(TransactionServiceGasEstimationPlan::Cached {
                gas_used: cached.gas_used,
                result_rlp: cached.result_rlp,
            });
        }
        Ok(TransactionServiceGasEstimationPlan::ExecuteEvm)
    }

    fn update_gas_prices(&mut self, gas_prices: Vec<U256>) {
        self.gas_price_oracle.update_from_gas_prices(gas_prices);
    }

    fn store_gas_estimation(
        &mut self,
        result: TransactionServiceGasEstimationResult,
    ) -> Result<bool> {
        self.sidecar
            .gas_estimation_cache_insert(
                result.hash,
                result.proposal_period,
                result.gas_used,
                result.result_rlp,
            )
            .context("TM_RUNTIME_GAS_ESTIMATION_CACHE_STORE")
    }

    fn initialize_recently_finalized(
        &mut self,
        period: u64,
        payloads: Vec<TransactionServicePayload>,
    ) -> Result<()> {
        let mut next_sidecar = self.sidecar.clone();
        let mut hashes = Vec::with_capacity(payloads.len());
        for payload in payloads {
            next_sidecar
                .insert_non_finalized(payload.hash, payload.transaction_rlp)
                .context("TM_RUNTIME_RECENT_FINALIZED_INIT_INSERT")?;
            hashes.push(payload.hash);
        }
        next_sidecar
            .apply_finalized_transition(period, hashes)
            .context("TM_RUNTIME_RECENT_FINALIZED_INIT_TRANSITION")?;
        self.sidecar = next_sidecar;
        Ok(())
    }

    fn remove_non_finalized(&mut self, hashes: Vec<H256>) -> Result<u64> {
        let mut existing = Vec::with_capacity(hashes.len());
        for hash in hashes {
            ensure!(
                !hash.is_zero(),
                "runtime sidecar removal hash cannot be zero"
            );
            if self.sidecar.contains_non_finalized(hash) {
                existing.push(hash);
            }
        }
        remove_non_finalized_transactions(&self.storage, existing.clone())
            .context("TM_RUNTIME_REMOVE_NON_FINALIZED_STORAGE")?;
        Ok(existing
            .into_iter()
            .filter(|hash| self.sidecar.remove_non_finalized(*hash))
            .count() as u64)
    }

    fn queue_block_finalized(&mut self, block_number: u64) -> Vec<H256> {
        self.queue.block_finalized(block_number)
    }

    fn filter_non_finalized(
        &self,
        requests: Vec<TransactionServiceFinalizedFilterRequest>,
    ) -> Result<FinalizedTransactionFilterPlan> {
        plan_exclude_finalized_transactions(
            requests
                .into_iter()
                .map(|request| FinalizedTransactionFilterFact {
                    input_index: request.input_index,
                    hash: request.hash,
                    in_recently_finalized_cache: self
                        .sidecar
                        .contains_recently_finalized(request.hash),
                })
                .collect(),
            |hash| transaction_finalized(&self.storage, hash).context("TM_FILTER_FINALIZED_LOOKUP"),
        )
    }

    fn verify_not_finalized(
        &self,
        facts: Vec<TransactionServiceVerifyNotFinalizedFact>,
    ) -> Result<TransactionServiceVerifyNotFinalizedOutcome> {
        let plan = plan_verify_not_finalized_transactions(
            facts
                .into_iter()
                .map(|fact| VerifyNotFinalizedTransactionFact {
                    input_index: fact.input_index,
                    hash: fact.hash,
                    transaction_nonce: fact.transaction_nonce,
                    sender_account_nonce: fact.sender_account_nonce,
                    in_recently_finalized_cache: self
                        .sidecar
                        .contains_recently_finalized(fact.hash),
                })
                .collect(),
            |hash| transaction_finalized(&self.storage, hash).context("TM_VERIFY_FINALIZED_LOOKUP"),
        )?;
        let Some(finalized) = plan.finalized else {
            return Ok(TransactionServiceVerifyNotFinalizedOutcome {
                is_finalized: false,
                input_index: 0,
                hash: H256::zero(),
                source: TM_VERIFY_NOT_FINALIZED_SOURCE_NONE,
            });
        };
        let source = if self.sidecar.contains_recently_finalized(finalized.hash) {
            TM_VERIFY_NOT_FINALIZED_SOURCE_RECENT_SIDECAR
        } else {
            TM_VERIFY_NOT_FINALIZED_SOURCE_STORAGE
        };
        Ok(TransactionServiceVerifyNotFinalizedOutcome {
            is_finalized: true,
            input_index: finalized.input_index,
            hash: finalized.hash,
            source,
        })
    }

    fn recover_non_finalized(&mut self) -> Result<u64> {
        let entries = load_non_finalized_recovery_entries(&self.storage)
            .context("TM_NONFINALIZED_RECOVERY_STORAGE")?;
        let mut recovered = Vec::with_capacity(entries.len());
        for entry in entries {
            if entry.finalized {
                continue;
            }
            let envelope = LegacyTransactionEnvelope::decode(&entry.trx_rlp)
                .context("TM_NONFINALIZED_RECOVERY_ENVELOPE_INSPECT")?;
            envelope
                .cost()
                .context("TM_NONFINALIZED_RECOVERY_ENVELOPE_INSPECT")?;
            ensure!(
                envelope.hash == entry.hash,
                "TM_NONFINALIZED_RECOVERY_HASH_MISMATCH"
            );
            ensure!(
                envelope.sender.is_some(),
                "TM_NONFINALIZED_RECOVERY_SENDER_MISSING"
            );
            recovered.push(TransactionManagerSidecarRecoveryEntry {
                hash: entry.hash,
                finalized: false,
                trx_rlp: entry.trx_rlp,
            });
        }
        let mut next_sidecar = self.sidecar.clone();
        let inserted = next_sidecar
            .insert_recovery_entries(recovered)
            .context("TM_RUNTIME_RECOVERY_INSERT")?;
        self.sidecar = next_sidecar;
        Ok(inserted as u64)
    }

    /// Prepares owner-scoped proposer packing from a validated queue/cache snapshot.
    ///
    /// Candidate payloads are decoded and cross-checked before any executor
    /// request escapes. Queue demotions and cache writes publish from cloned
    /// next state only after every effect succeeds. An estimate-needed result
    /// retains the exact packing owner; immediate results leave it inactive.
    pub(crate) fn prepare_proposer_pack(
        &mut self,
        owner: TransactionPackingOwner,
        params: crate::dag_service::DagProposerPackParameters,
        min_transaction_gas: u64,
        estimate_gas_limit: u64,
        last_block_number: u64,
    ) -> Result<TransactionServiceProposerPackPrepared> {
        let outcome = self.prepare_proposer_pack_for_owner(
            owner,
            params.weight_limit,
            min_transaction_gas,
            params.proposal_period,
            estimate_gas_limit,
            last_block_number,
            params.total_transaction_shards,
            params.node_transaction_shard,
            params.shard_period_interval,
        )?;
        Ok(TransactionServiceProposerPackPrepared {
            request_estimates: outcome.request_estimates,
            selected_transactions: outcome.selected_transactions,
        })
    }

    /// Finalizes an exact owner-scoped estimate sequence and publishes its effects.
    ///
    /// Count, ordering, hash, or owner mismatches return without publishing
    /// queue/cache state. Successful demotion and cache effects are precomputed
    /// on clones and then installed together before selected canonical payloads
    /// are returned to the application root.
    pub(crate) fn finalize_proposer_pack(
        &mut self,
        owner: TransactionPackingOwner,
        estimates: Vec<TransactionPackingEstimate>,
    ) -> Result<TransactionServiceProposerPackFinalized> {
        let outcome = self.finalize_proposer_pack_for_owner(owner, estimates)?;
        Ok(TransactionServiceProposerPackFinalized {
            selected_transactions: outcome.selected_transactions,
        })
    }

    /// Prepares owner-scoped proposer packing with compatibility demotion metadata.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_proposer_pack_for_owner(
        &mut self,
        owner: TransactionPackingOwner,
        weight_limit: u64,
        min_transaction_gas: u64,
        proposal_period: u64,
        estimate_gas_limit: u64,
        last_block_number: u64,
        total_shards: u16,
        node_shard: u16,
        shard_period_interval: u64,
    ) -> Result<TransactionServiceCompatibilityPackPrepared> {
        let candidate_limit = TransactionPackingService::candidate_limit(
            weight_limit,
            min_transaction_gas,
            total_shards,
            node_shard,
            shard_period_interval,
        )?;
        let candidates = self
            .queue
            .ordered_transactions(candidate_limit)
            .into_iter()
            .map(|entry| {
                let cached_gas_used = self
                    .sidecar
                    .gas_estimation_cache_get(entry.hash, proposal_period)
                    .context("TM_RUNTIME_PACK_GAS_ESTIMATION_CACHE_GET")?
                    .map(|cached| cached.gas_used);
                Ok((entry, cached_gas_used))
            })
            .collect::<Result<Vec<_>>>()?;
        let step = self
            .transaction_packing
            .prepare(TransactionPackingRequest {
                owner,
                weight_limit,
                min_transaction_gas,
                proposal_period,
                estimate_gas_limit,
                last_block_number,
                total_shards,
                node_shard,
                shard_period_interval,
                candidates: candidates
                    .into_iter()
                    .map(|(entry, cached_gas_used)| TransactionPackingCandidate {
                        entry,
                        cached_gas_used,
                    })
                    .collect(),
            })
            .context("TM_RUNTIME_PACK_PREPARE")?;

        let request_estimates = match step
            .request_estimates
            .iter()
            .cloned()
            .map(transaction_estimate_request)
            .collect::<Result<Vec<_>>>()
        {
            Ok(requests) => requests,
            Err(error) => {
                let _ = self.transaction_packing.abort(owner);
                return Err(error);
            }
        };
        let (selected_transactions, demoted_hashes) =
            match apply_packing_step_effects(self, &step, "TM_RUNTIME_PACK_PREPARE") {
                Ok(outcome) => outcome,
                Err(error) => {
                    let _ = self.transaction_packing.abort(owner);
                    return Err(error);
                }
            };
        if !step.request_estimates.is_empty() {
            self.transaction_packing
                .acknowledge_demotions(owner, demoted_hashes.to_vec())?;
        }
        let mut ordered_demoted_hashes = step.acknowledged_demotions.clone();
        ordered_demoted_hashes.extend(demoted_hashes);

        Ok(TransactionServiceCompatibilityPackPrepared {
            request_estimates,
            selected_transactions,
            demoted_hashes: ordered_demoted_hashes,
            stopped: step.stopped,
        })
    }

    /// Finalizes an owner-scoped estimate sequence and returns compatibility metadata.
    pub(crate) fn finalize_proposer_pack_for_owner(
        &mut self,
        owner: TransactionPackingOwner,
        estimates: Vec<TransactionPackingEstimate>,
    ) -> Result<TransactionServiceCompatibilityPackFinalized> {
        let step = self
            .transaction_packing
            .finalize(owner, estimates)
            .context("TM_RUNTIME_PACK_FINALIZE")?;
        let (selected_transactions, demoted_hashes) =
            apply_packing_step_effects(self, &step, "TM_RUNTIME_PACK_FINALIZE")?;
        let mut ordered_demoted_hashes = step.acknowledged_demotions;
        ordered_demoted_hashes.extend(demoted_hashes);
        Ok(TransactionServiceCompatibilityPackFinalized {
            selected_transactions,
            demoted_hashes: ordered_demoted_hashes,
            stopped: step.stopped,
        })
    }

    /// Aborts one active owner session from Rust proposer packing.
    pub(crate) fn abort_proposer_pack(&mut self, owner: TransactionPackingOwner) -> Result<bool> {
        self.transaction_packing.abort(owner)
    }
}

fn transaction_estimate_request(
    request: TransactionPackingEstimateRequest,
) -> Result<TransactionServiceEstimateRequest> {
    let entry = request.entry;
    let envelope = LegacyTransactionEnvelope::decode(&entry.rlp)
        .context("TM_RUNTIME_PACK_CANDIDATE_ENVELOPE_INSPECT_FAILED")?;
    envelope
        .cost()
        .context("TM_RUNTIME_PACK_CANDIDATE_ENVELOPE_INSPECT_FAILED")?;
    ensure!(
        envelope.hash == entry.hash,
        "TM_RUNTIME_PACK_CANDIDATE_HASH_MISMATCH"
    );
    let sender = envelope
        .sender
        .context("TM_RUNTIME_PACK_CANDIDATE_SENDER_MISSING")?;
    ensure!(
        sender == entry.sender,
        "TM_RUNTIME_PACK_CANDIDATE_SENDER_MISMATCH"
    );
    ensure!(
        envelope.nonce == entry.nonce,
        "TM_RUNTIME_PACK_CANDIDATE_NONCE_MISMATCH"
    );
    ensure!(
        envelope.gas_price == entry.gas_price,
        "TM_RUNTIME_PACK_CANDIDATE_GAS_PRICE_MISMATCH"
    );
    ensure!(
        envelope.gas == entry.gas,
        "TM_RUNTIME_PACK_CANDIDATE_GAS_MISMATCH"
    );
    ensure!(
        envelope.data.len() as u64 == entry.data_size,
        "TM_RUNTIME_PACK_CANDIDATE_DATA_SIZE_MISMATCH"
    );
    Ok(TransactionServiceEstimateRequest {
        hash: entry.hash,
        transaction_rlp: entry.rlp,
        declared_gas: entry.gas,
        sender,
        nonce: entry.nonce,
        gas_price: entry.gas_price,
        gas: entry.gas,
        receiver: envelope.receiver,
        value: envelope.value,
        data: envelope.data,
    })
}

fn bounded_transaction_view_count(requests_len: usize, max_count: u64) -> usize {
    match max_count {
        0 => requests_len,
        _ => (max_count.min(requests_len as u64)) as usize,
    }
}

fn transaction_service_transaction_view_source_from_stored_transaction_source(
    source: u8,
) -> Result<u8> {
    match source {
        STORED_TRANSACTION_SOURCE_MISSING => Ok(TM_TRANSACTION_VIEW_SOURCE_MISSING),
        STORED_TRANSACTION_SOURCE_PENDING => Ok(TM_TRANSACTION_VIEW_SOURCE_STORAGE_PENDING),
        STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR => {
            Ok(TM_TRANSACTION_VIEW_SOURCE_STORAGE_FINALIZED_REGULAR)
        }
        STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM => {
            Ok(TM_TRANSACTION_VIEW_SOURCE_STORAGE_FINALIZED_SYSTEM)
        }
        _ => Err(anyhow!("TM_TRANSACTION_VIEW_UNKNOWN_STORED_SOURCE")),
    }
}

fn transaction_service_transaction_view_source_from_sidecar_transaction_source(
    source: u8,
) -> Result<u8> {
    match source {
        crate::transaction_manager::TransactionManagerSidecarLookup::SOURCE_NON_FINALIZED => {
            Ok(TM_TRANSACTION_VIEW_SOURCE_NON_FINALIZED_SIDECAR)
        }
        crate::transaction_manager::TransactionManagerSidecarLookup::SOURCE_RECENTLY_FINALIZED => {
            Ok(TM_TRANSACTION_VIEW_SOURCE_RECENTLY_FINALIZED_SIDECAR)
        }
        _ => Err(anyhow!("TM_TRANSACTION_VIEW_UNKNOWN_SIDECAR_SOURCE")),
    }
}

fn transaction_service_load_stored_transactions_from_storage(
    storage: &Storage,
    requests: Vec<TransactionServiceStoredTransactionRequest>,
) -> Result<Vec<TransactionServiceStoredTransactionLookup>> {
    let requests = requests
        .into_iter()
        .map(|request| StoredTransactionLookupRequest {
            input_index: request.input_index,
            hash: H256::from(request.hash),
        })
        .collect();

    load_stored_transactions(storage, requests)
        .context("TM_TRANSACTION_RLP_STORAGE_LOOKUP")?
        .into_iter()
        .map(|lookup| {
            Ok(TransactionServiceStoredTransactionLookup {
                hash: lookup.hash.0,
                found: lookup.found,
                source: lookup.source,
                old_finalized: false,
                tx_rlp: lookup.tx_rlp,
            })
        })
        .collect()
}

fn transaction_service_load_proposal_transactions_with_account_nonce_facts_from_storage(
    storage: &Storage,
    _proposal_period: u64,
    account_nonce_facts: Vec<TransactionServiceAccountNonceFact>,
    requests: Vec<TransactionServiceStoredTransactionRequest>,
    require_finalized_account_nonce_fact: bool,
) -> Result<Vec<TransactionServiceStoredTransactionLookup>> {
    let account_nonce_facts: HashMap<H160, (bool, U256)> = account_nonce_facts
        .into_iter()
        .map(|fact| {
            (
                H160::from(fact.sender),
                (
                    fact.account_found,
                    U256::from_big_endian(&fact.account_nonce),
                ),
            )
        })
        .collect();

    let lookups = transaction_service_load_stored_transactions_from_storage(storage, requests)?;

    lookups
        .into_iter()
        .map(|mut lookup| {
            if !lookup.found || !is_finalized_stored_transaction_source(lookup.source) {
                return Ok(lookup);
            }

            let expected_hash = H256::from(lookup.hash);
            ensure!(
                keccak256(&lookup.tx_rlp) == expected_hash,
                "TM_PROPOSAL_TRANSACTION_HASH_MISMATCH"
            );
            let identity = inspect_stored_transaction_identity(&lookup.tx_rlp, lookup.source)
                .context("TM_PROPOSAL_TRANSACTION_IDENTITY_INSPECT_FAILED")?;
            let account_nonce = account_nonce_facts
                .get(&H160::from(identity.sender))
                .copied();
            ensure!(
                account_nonce.is_some() || !require_finalized_account_nonce_fact,
                "TM_PROPOSAL_FINALIZED_ACCOUNT_NONCE_FACT_MISSING"
            );
            let account_nonce = account_nonce.unwrap_or((false, U256::zero()));

            if account_nonce.0 && account_nonce.1 > identity.nonce {
                lookup.found = false;
                lookup.old_finalized = true;
                lookup.tx_rlp.clear();
            }
            Ok(lookup)
        })
        .collect()
}

fn is_finalized_stored_transaction_source(source: u8) -> bool {
    source == STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR
        || source == STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM
}

fn inspect_stored_transaction_identity(
    tx_rpl: &[u8],
    source: u8,
) -> Result<StoredTransactionIdentity> {
    let tx = if source == STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM {
        LegacyTransactionEnvelope::decode_system(tx_rpl)
    } else {
        LegacyTransactionEnvelope::decode(tx_rpl)
    }
    .context("TM_TRANSACTION_RLP_PARSE_FAILED")?;
    let sender = tx
        .sender
        .ok_or_else(|| anyhow!("TM_TRANSACTION_RLP_SENDER_RECOVERY_FAILED"))?;

    Ok(StoredTransactionIdentity {
        sender: sender.0,
        nonce: tx.nonce,
    })
}

fn transaction_service_runtime_lookup_transaction_views_inner(
    runtime: &TransactionServiceState,
    requests: Vec<TransactionServiceTransactionViewRequest>,
    max_count: u64,
    transaction_lookup: impl FnOnce(
        Vec<TransactionServiceStoredTransactionRequest>,
    ) -> Result<Vec<TransactionServiceStoredTransactionLookup>>,
) -> Result<TransactionServiceTransactionViewPlan> {
    let total_requests = requests.len();
    let requested_count = bounded_transaction_view_count(total_requests, max_count) as u64;
    let mut views = Vec::with_capacity(requested_count as usize);
    let mut sidecar_requests = Vec::new();
    let mut sidecar_view_indexes = Vec::new();

    for request in requests.into_iter().take(requested_count as usize) {
        let hash = H256::from(request.hash);
        let queue_view = runtime.queue.transaction(hash);

        let mut view = TransactionServiceTransactionView {
            input_index: request.input_index,
            hash: request.hash,
            found: false,
            source: TM_TRANSACTION_VIEW_SOURCE_MISSING,
            old_finalized: false,
            tx_rlp: Vec::new(),
        };

        if let Some(entry) = queue_view {
            view.found = true;
            view.source = TM_TRANSACTION_VIEW_SOURCE_QUEUE;
            view.tx_rlp = entry.rlp;
            views.push(view);
            continue;
        }

        let view_index = views.len();
        views.push(view);
        sidecar_requests.push((request.input_index, hash));
        sidecar_view_indexes.push(view_index);
    }

    if !sidecar_requests.is_empty() {
        let sidecar_lookups = runtime
            .sidecar
            .lookup_payloads_ordered(
                sidecar_requests
                    .into_iter()
                    .map(|request| (request.0, request.1))
                    .collect(),
            )
            .context("TM_RUNTIME_TRANSACTION_VIEW_SIDECAR_LOOKUP")?;
        ensure!(
            sidecar_lookups.len() == sidecar_view_indexes.len(),
            "TM_RUNTIME_TRANSACTION_VIEW_SIDECAR_RESULT_COUNT_MISMATCH"
        );

        let mut storage_requests = Vec::new();
        let mut storage_view_indexes = Vec::new();
        for (idx, lookup) in sidecar_lookups.into_iter().enumerate() {
            let view_index = sidecar_view_indexes[idx];
            if lookup.found {
                let source =
                    transaction_service_transaction_view_source_from_sidecar_transaction_source(
                        lookup.source,
                    )
                    .context("TM_RUNTIME_TRANSACTION_VIEW_SIDECAR_SOURCE")?;
                views[view_index].found = true;
                views[view_index].source = source;
                views[view_index].tx_rlp = lookup.trx_rlp;
            } else {
                storage_requests.push(TransactionServiceStoredTransactionRequest {
                    input_index: lookup.input_index,
                    hash: lookup.hash.0,
                });
                storage_view_indexes.push(view_index);
            }
        }

        if !storage_requests.is_empty() {
            let stored_lookups = transaction_lookup(storage_requests)?;
            ensure!(
                stored_lookups.len() == storage_view_indexes.len(),
                "TM_RUNTIME_TRANSACTION_VIEW_STORED_RESULT_COUNT_MISMATCH"
            );
            for (idx, lookup) in stored_lookups.into_iter().enumerate() {
                let view_index = storage_view_indexes[idx];
                views[view_index].found = lookup.found;
                views[view_index].old_finalized = lookup.old_finalized;
                views[view_index].tx_rlp = lookup.tx_rlp;
                views[view_index].source =
                    transaction_service_transaction_view_source_from_stored_transaction_source(
                        lookup.source,
                    )
                    .context("TM_RUNTIME_TRANSACTION_VIEW_STORED_SOURCE")?;
            }
        }
    }

    Ok(TransactionServiceTransactionViewPlan {
        requested_count,
        complete: requested_count == total_requests as u64,
        views,
    })
}

fn keccak256(data: &[u8]) -> H256 {
    use tiny_keccak::{Hasher, Keccak};

    let mut output = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(data);
    hasher.finalize(&mut output);
    H256::from(output)
}

fn apply_packing_step_effects(
    runtime: &mut TransactionServiceState,
    step: &crate::transaction_packing_service::TransactionPackingStep,
    cache_context: &'static str,
) -> Result<(Vec<TransactionPackingSelection>, Vec<H256>)> {
    let mut next_queue = runtime.queue.clone();
    let mut next_sidecar = runtime.sidecar.clone();
    let mut demoted_hashes = Vec::new();
    for effect in &step.effects {
        match effect {
            TransactionPackingEffect::Demote(intent) => {
                let outcome = next_queue.demote(intent.hash, intent.last_block_number);
                if matches!(outcome.status, TransactionQueueDemoteStatus::Demoted) {
                    demoted_hashes.push(intent.hash);
                }
            }
            TransactionPackingEffect::CacheInsert(intent) => {
                next_sidecar
                    .gas_estimation_cache_insert(
                        intent.hash,
                        intent.proposal_period,
                        intent.gas_used,
                        intent.result_rlp.clone(),
                    )
                    .context(cache_context)?;
            }
        }
    }
    runtime.queue = next_queue;
    runtime.sidecar = next_sidecar;
    Ok((step.selected.clone(), demoted_hashes))
}

/// Plans accepted DAG transactions without mutating storage or live state.
pub(crate) fn prepare_dag_transactions(
    runtime: &TransactionServiceState,
    facts: Vec<DagTransactionSaveInput>,
) -> Result<PreparedDagTransactionSave> {
    let plan = plan_transactions_from_dag_block(
        facts
            .into_iter()
            .map(|fact| DagTransactionSaveFact {
                input_index: fact.input_index,
                hash: fact.hash,
                trx_rlp: fact.transaction_rlp,
                transaction_nonce: fact.transaction_nonce,
                sender_account_nonce: fact.sender_account_nonce,
                in_non_finalized_cache: runtime.sidecar.contains_non_finalized(fact.hash),
                in_recently_finalized_cache: runtime.sidecar.contains_recently_finalized(fact.hash),
            })
            .collect(),
        runtime.sidecar.transaction_count(),
        |hash| {
            transaction_finalized(runtime.storage.as_ref(), hash)
                .context("TM_DAG_TX_FINALIZED_LOOKUP_FAILED")
        },
    )?;

    let accepted = plan
        .accepted_transactions
        .iter()
        .map(|payload| DagTransactionSaveAccepted {
            hash: payload.hash,
            erased_from_queue: false,
        })
        .collect();
    let accepted_payloads = plan
        .accepted_transactions
        .into_iter()
        .map(|payload| NonFinalizedTransactionStoragePayload {
            hash: payload.hash,
            trx_rlp: payload.trx_rlp,
        })
        .collect();
    Ok(PreparedDagTransactionSave {
        accepted,
        accepted_payloads,
        target_transaction_count: plan.target_transaction_count,
    })
}

/// Appends prepared DAG transaction writes to a caller-owned atomic batch.
pub(crate) fn append_prepared_dag_transactions(
    storage: &Storage,
    batch: &mut StorageWriteBatch,
    prepared: &PreparedDagTransactionSave,
) -> Result<()> {
    if prepared.accepted_payloads.is_empty() {
        return Ok(());
    }
    append_non_finalized_transactions_to_batch(
        storage,
        batch,
        prepared.accepted_payloads.clone(),
        prepared.target_transaction_count,
    )
}

/// Precomputes queue and sidecar state for post-commit publication.
pub(crate) fn prepare_dag_transaction_publication(
    runtime: &TransactionServiceState,
    prepared: &PreparedDagTransactionSave,
) -> Result<PreparedDagTransactionPublication> {
    let mut queue = runtime.queue.clone();
    let mut sidecar = runtime.sidecar.clone();
    let mut accepted = prepared.accepted.clone();
    for (accepted_entry, payload) in accepted.iter_mut().zip(prepared.accepted_payloads.iter()) {
        accepted_entry.erased_from_queue = queue.erase(payload.hash);
        sidecar.insert_non_finalized(payload.hash, payload.trx_rlp.clone())?;
    }
    sidecar.set_transaction_count(prepared.target_transaction_count);
    Ok(PreparedDagTransactionPublication {
        queue,
        sidecar,
        outcome: DagTransactionSaveOutcome { accepted },
    })
}

/// Publishes a prevalidated transaction transition after shared-batch commit.
pub(crate) fn publish_dag_transactions(
    runtime: &mut TransactionServiceState,
    publication: PreparedDagTransactionPublication,
) -> DagTransactionSaveOutcome {
    runtime.queue = publication.queue;
    runtime.sidecar = publication.sidecar;
    publication.outcome
}

/// Removes live non-finalized sidecars after native DAG cleanup committed storage.
///
/// The operation is infallible and performs no storage writes. Hashes absent
/// from the sidecar are ignored so restart and duplicate cleanup remain safe.
pub(crate) fn remove_non_finalized_sidecars_after_dag_commit(
    runtime: &mut TransactionServiceState,
    hashes: &[H256],
) -> u64 {
    hashes.iter().fold(0, |removed, hash| {
        removed + u64::from(runtime.sidecar.remove_non_finalized(*hash))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use ethereum_types::H160;
    use ethereum_types::H256;
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_storage::{Config, Storage};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tiny_keccak::{Hasher, Keccak};

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn build_service_with_defaults(
        status_field: Option<u64>,
        initial_queue_size: usize,
        gas_pricer_config: GasPricerConfig,
    ) -> Result<(TransactionService, std::path::PathBuf)> {
        let temp_dir = unique_temp_dir("rustaxa_consensus_transaction_service_test");
        let storage = Arc::new(Storage::new(Config::new(temp_dir.clone()))?);
        if let Some(count) = status_field {
            storage
                .metadata()
                .write_status_field(StatusField::TrxCount as u8, count)?;
        }
        let service = TransactionService::restore(
            storage,
            TransactionServiceConfig {
                queue_max_size: initial_queue_size,
                gas_pricer_config,
                proposal_dag_gas_limit: 1_000_000,
            },
        )?;
        Ok((service, temp_dir))
    }

    #[test]
    fn save_dag_transactions_commits_before_publishing_queue_and_sidecar() -> Result<()> {
        let (service, temp_dir) = build_service_with_defaults(
            Some(7),
            16,
            GasPricerConfig {
                percentile: 50,
                minimum_price: U256::one(),
                history_blocks: 10,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        let hash = H256::repeat_byte(0x31);
        let transaction_rlp = vec![0xC1, 0x80];
        {
            let mut runtime = service.lock()?;
            runtime.queue.insert(
                TransactionQueueEntry {
                    hash,
                    sender: H160::repeat_byte(0x22),
                    nonce: U256::from(5),
                    gas_price: U256::from(2),
                    gas: 21_000,
                    data_size: 0,
                    rlp: transaction_rlp.clone(),
                    last_block_number: 0,
                },
                true,
            )?;
        }

        let outcome = service.save_dag_transactions(vec![DagTransactionSaveInput {
            input_index: 0,
            hash,
            transaction_rlp: transaction_rlp.clone(),
            transaction_nonce: U256::from(5),
            sender_account_nonce: U256::from(4),
        }])?;

        assert_eq!(outcome.accepted.len(), 1);
        assert_eq!(outcome.accepted[0].hash, hash);
        assert!(outcome.accepted[0].erased_from_queue);
        {
            let runtime = service.lock()?;
            assert!(!runtime.queue.contains(hash));
            assert!(runtime.sidecar.contains_non_finalized(hash));
            assert_eq!(runtime.sidecar.transaction_count(), 8);
            assert_eq!(
                runtime.storage.transaction().rlp(hash)?,
                Some(transaction_rlp)
            );
            assert_eq!(
                runtime
                    .storage
                    .metadata()
                    .status_field(StatusField::TrxCount as u8)?,
                8
            );
        }

        drop(service);
        std::fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    #[test]
    fn save_dag_transactions_commit_failure_publishes_nothing() -> Result<()> {
        let (service, temp_dir) = build_service_with_defaults(
            Some(7),
            16,
            GasPricerConfig {
                percentile: 50,
                minimum_price: U256::one(),
                history_blocks: 10,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        let hash = H256::repeat_byte(0x32);
        let transaction_rlp = vec![0xC1, 0x80];
        {
            let mut runtime = service.lock()?;
            runtime.queue.insert(
                TransactionQueueEntry {
                    hash,
                    sender: H160::repeat_byte(0x23),
                    nonce: U256::from(5),
                    gas_price: U256::from(2),
                    gas: 21_000,
                    data_size: 0,
                    rlp: transaction_rlp.clone(),
                    last_block_number: 0,
                },
                true,
            )?;
        }

        let error = service
            .save_dag_transactions_with_committer(
                vec![DagTransactionSaveInput {
                    input_index: 0,
                    hash,
                    transaction_rlp,
                    transaction_nonce: U256::from(5),
                    sender_account_nonce: U256::from(4),
                }],
                |_storage, _batch| Err(anyhow!("injected commit failure")),
            )
            .expect_err("injected commit failure must be returned");
        assert!(format!("{error:#}").contains("TM_DAG_TX_BATCH_COMMIT"));

        {
            let runtime = service.lock()?;
            assert!(runtime.queue.contains(hash));
            assert!(!runtime.sidecar.contains_non_finalized(hash));
            assert_eq!(runtime.sidecar.transaction_count(), 7);
            assert_eq!(runtime.storage.transaction().rlp(hash)?, None);
            assert_eq!(
                runtime
                    .storage
                    .metadata()
                    .status_field(StatusField::TrxCount as u8)?,
                7
            );
        }

        drop(service);
        std::fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    fn append_gas_price_transaction(stream: &mut RlpStream, gas_price: u64) {
        stream.begin_list(9);
        stream.append(&0u64);
        stream.append(&gas_price);
        stream.append(&21_000u64);
        stream.append_empty_data();
        stream.append(&0u64);
        stream.append_empty_data();
        stream.append(&27u64);
        stream.append(&1u64);
        stream.append(&1u64);
    }

    fn keccak256(bytes: &[u8]) -> H256 {
        let mut output = [0u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(bytes);
        hasher.finalize(&mut output);
        H256::from(output)
    }

    fn transaction_service_view_request(
        input_index: u64,
        hash: u64,
    ) -> TransactionServiceTransactionViewRequest {
        TransactionServiceTransactionViewRequest {
            input_index,
            hash: H256::from_low_u64_be(hash).0,
        }
    }

    fn period_data_with_transaction_rlps(transactions: &[Vec<u8>]) -> Vec<u8> {
        let mut txs = RlpStream::new_list(transactions.len());
        for tx in transactions {
            txs.append_raw(tx, 1);
        }
        let mut stream = RlpStream::new_list(5);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&txs.out(), 1);
        stream.append_raw(&[0xC0], 1);
        stream.out().to_vec()
    }

    fn signed_legacy_transaction_rlp(signing_key: &SigningKey) -> Vec<u8> {
        signed_legacy_transaction_rlp_with_fields(
            signing_key,
            U256::from(1),
            U256::from(2),
            21_000,
            U256::from(3),
        )
    }

    fn signed_legacy_transaction_rlp_with_fields(
        signing_key: &SigningKey,
        nonce: U256,
        gas_price: U256,
        gas: u64,
        value: U256,
    ) -> Vec<u8> {
        let chain_id = 2999_u64;
        let mut unsigned = RlpStream::new_list(9);
        unsigned.append(&nonce);
        unsigned.append(&gas_price);
        unsigned.append(&gas);
        unsigned.append(&H160::repeat_byte(0x44));
        unsigned.append(&value);
        unsigned.append(&Vec::<u8>::new());
        unsigned.append(&U256::from(chain_id));
        unsigned.append(&U256::zero());
        unsigned.append(&U256::zero());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(keccak256(&unsigned.out()).as_bytes())
            .expect("test transaction signing must succeed");
        let signature = signature.to_bytes();
        let mut signed = RlpStream::new_list(9);
        signed.append(&nonce);
        signed.append(&gas_price);
        signed.append(&gas);
        signed.append(&H160::repeat_byte(0x44));
        signed.append(&value);
        signed.append(&Vec::<u8>::new());
        signed.append(&U256::from(
            chain_id * 2 + 35 + u64::from(recovery_id.to_byte()),
        ));
        signed.append(&U256::from_big_endian(&signature[..32]));
        signed.append(&U256::from_big_endian(&signature[32..]));
        signed.out().to_vec()
    }

    fn seed_gas_price_history(storage: &Storage, blocks: &[(u64, &[u64])]) -> Result<()> {
        for &(period, prices) in blocks {
            let mut period_rlp = RlpStream::new_list(4);
            period_rlp.append_empty_data();
            period_rlp.append_empty_data();
            period_rlp.begin_list(0);
            period_rlp.begin_list(prices.len());
            for &gas_price in prices {
                append_gas_price_transaction(&mut period_rlp, gas_price);
            }
            storage.period().write(period, &period_rlp.out())?;
        }
        Ok(())
    }

    fn seed_last_finalized_block(storage: &Storage, block: u64) -> Result<()> {
        storage
            .final_chain()
            .write_block_header(block, H256::zero(), &[], &[])
            .context("seed_last_finalized_block")
    }

    #[test]
    fn restore_defaults_missing_count_to_zero() -> Result<()> {
        let (service, path) = build_service_with_defaults(
            None,
            16,
            GasPricerConfig {
                percentile: 50,
                minimum_price: ethereum_types::U256::one(),
                history_blocks: 10,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        let runtime = service.lock()?;

        assert_eq!(runtime.sidecar.transaction_count(), 0);

        drop(runtime);
        drop(service);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn restore_reads_persisted_count() -> Result<()> {
        let (service, path) = build_service_with_defaults(
            Some(73),
            16,
            GasPricerConfig {
                percentile: 50,
                minimum_price: ethereum_types::U256::one(),
                history_blocks: 10,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        let runtime = service.lock()?;

        assert_eq!(runtime.sidecar.transaction_count(), 73);

        drop(runtime);
        drop(service);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn restored_owner_keeps_storage_queue_sidecar_and_packing_coherent() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_transaction_service_coherence");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let service = TransactionService::restore(
            storage.clone(),
            TransactionServiceConfig {
                queue_max_size: 8,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: ethereum_types::U256::one(),
                    history_blocks: 0,
                    is_light_node: false,
                    blocks_gas_pricer: false,
                },
                proposal_dag_gas_limit: 42_000,
            },
        )?;
        let runtime = service.lock()?;

        assert!(Arc::ptr_eq(&runtime.storage, &storage));
        assert_eq!(runtime.sidecar.transaction_count(), 0);
        assert_eq!(runtime.queue.size(), 0);
        assert!(!runtime.transaction_packing.is_active()?);
        assert_eq!(runtime.proposal_dag_gas_limit, 42_000);
        assert!(runtime.last_drop_observed.is_none());

        drop(runtime);
        drop(service);
        drop(storage);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn gas_pricer_updates_history_and_ignores_empty_blocks() -> Result<()> {
        let temp_dir = unique_temp_dir("rustaxa_consensus_transaction_service_test_gas_oracle");
        let storage = Arc::new(Storage::new(Config::new(temp_dir.clone()))?);
        let config = GasPricerConfig {
            percentile: 50,
            minimum_price: ethereum_types::U256::one(),
            history_blocks: 10,
            is_light_node: false,
            blocks_gas_pricer: true,
        };
        let service = TransactionService::restore(
            storage,
            TransactionServiceConfig {
                queue_max_size: 16,
                gas_pricer_config: config,
                proposal_dag_gas_limit: 1_000_000,
            },
        )?;
        let mut runtime = service.lock()?;

        runtime
            .gas_price_oracle
            .update_from_gas_prices(std::iter::empty::<ethereum_types::U256>());
        assert_eq!(
            runtime.gas_price_oracle.bid(),
            ethereum_types::U256::from(1_u64)
        );

        for price in [1_u64, 2, 3, 4, 5] {
            runtime
                .gas_price_oracle
                .update_from_gas_prices([ethereum_types::U256::from(price)]);
        }
        assert_eq!(
            runtime.gas_price_oracle.bid(),
            ethereum_types::U256::from(3_u64)
        );

        std::fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    #[test]
    fn restore_restores_gas_history_and_restarts() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_transaction_service_restart");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        seed_gas_price_history(&storage, &[(2, &[9, 5]), (1, &[8])])?;
        seed_last_finalized_block(&storage, 2)?;

        for _ in 0..2 {
            let service = TransactionService::restore(
                storage.clone(),
                TransactionServiceConfig {
                    queue_max_size: 16,
                    gas_pricer_config: GasPricerConfig {
                        percentile: 50,
                        minimum_price: ethereum_types::U256::one(),
                        history_blocks: 10,
                        is_light_node: false,
                        blocks_gas_pricer: true,
                    },
                    proposal_dag_gas_limit: 1_000_000,
                },
            )?;
            let runtime = service.lock()?;
            assert_eq!(
                runtime.gas_price_oracle.bid(),
                ethereum_types::U256::from(5_u64)
            );
        }

        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn light_mode_stops_without_full_missing_history() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_transaction_service_light_history");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        seed_gas_price_history(&storage, &[(2, &[9])])?;
        seed_last_finalized_block(&storage, 3)?;

        let light = TransactionService::restore(
            storage.clone(),
            TransactionServiceConfig {
                queue_max_size: 16,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: ethereum_types::U256::from(7_u64),
                    history_blocks: 10,
                    is_light_node: true,
                    blocks_gas_pricer: true,
                },
                proposal_dag_gas_limit: 1_000_000,
            },
        )?;
        let runtime = light.lock()?;
        assert_eq!(
            runtime.gas_price_oracle.bid(),
            ethereum_types::U256::from(7_u64)
        );

        let err = TransactionService::restore(
            storage,
            TransactionServiceConfig {
                queue_max_size: 16,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: ethereum_types::U256::one(),
                    history_blocks: 10,
                    is_light_node: false,
                    blocks_gas_pricer: true,
                },
                proposal_dag_gas_limit: 1_000_000,
            },
        )
        .err()
        .expect("full-node history restore must reject missing period data");
        assert!(
            format!("{err:#}").contains("missing finalized transactions for block 3"),
            "unexpected restoration error: {err:#}"
        );

        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn pool_bid_respects_queue_limit_and_oracle_floor() -> Result<()> {
        let temp_dir = unique_temp_dir("rustaxa_consensus_transaction_service_test_pool_bid");
        let storage = Arc::new(Storage::new(Config::new(temp_dir.clone()))?);
        let service = TransactionService::restore(
            storage,
            TransactionServiceConfig {
                queue_max_size: 100,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: ethereum_types::U256::from(4_u64),
                    history_blocks: 0,
                    is_light_node: false,
                    blocks_gas_pricer: false,
                },
                proposal_dag_gas_limit: 42_000,
            },
        )?;
        {
            let mut runtime = service.lock()?;
            runtime
                .queue
                .insert(
                    crate::transaction_queue::TransactionQueueEntry {
                        hash: ethereum_types::H256::from_low_u64_be(1),
                        sender: ethereum_types::H160::from_low_u64_be(1),
                        nonce: ethereum_types::U256::zero(),
                        gas_price: ethereum_types::U256::from(2_u64),
                        gas: 21_000,
                        data_size: 0,
                        rlp: vec![1],
                        last_block_number: 0,
                    },
                    true,
                )
                .expect("queue insert should work");
            runtime
                .queue
                .insert(
                    crate::transaction_queue::TransactionQueueEntry {
                        hash: ethereum_types::H256::from_low_u64_be(2),
                        sender: ethereum_types::H160::from_low_u64_be(2),
                        nonce: ethereum_types::U256::zero(),
                        gas_price: ethereum_types::U256::from(4_u64),
                        gas: 21_000,
                        data_size: 0,
                        rlp: vec![2],
                        last_block_number: 0,
                    },
                    true,
                )
                .expect("queue insert should work");

            let pool_price = runtime.queue.min_gas_price_for_block_inclusion(42_000);
            assert_eq!(pool_price, ethereum_types::U256::from(3_u64));
            assert_eq!(
                runtime.gas_price_oracle.configured_bid(pool_price),
                ethereum_types::U256::from(4_u64)
            );
            runtime.sidecar.gas_estimation_cache_insert(
                H256::from_low_u64_be(2),
                7,
                19_000,
                vec![0xC1],
            )?;
        }
        assert_eq!(
            U256::from_big_endian(&service.gas_price_bid()?),
            U256::from(4_u64)
        );
        assert_eq!(service.queue_size()?, 2);
        assert_eq!(service.queue_transaction_groups()?.len(), 2);
        assert_eq!(service.queue_proposable_accounts()?.len(), 2);
        assert!(!service.queue_non_proposable_over_limit()?);
        assert_eq!(
            service.plan_gas_estimation(TransactionServiceGasEstimationRequest {
                hash: H256::from_low_u64_be(1),
                declared_gas: 21_000,
                proposal_period: 7,
                estimate_gas_limit: 30_000,
            })?,
            TransactionServiceGasEstimationPlan::Declared { gas_used: 21_000 }
        );
        assert_eq!(
            service.plan_gas_estimation(TransactionServiceGasEstimationRequest {
                hash: H256::from_low_u64_be(2),
                declared_gas: 50_000,
                proposal_period: 7,
                estimate_gas_limit: 30_000,
            })?,
            TransactionServiceGasEstimationPlan::Cached {
                gas_used: 19_000,
                result_rlp: vec![0xC1],
            }
        );
        assert_eq!(
            service.plan_gas_estimation(TransactionServiceGasEstimationRequest {
                hash: H256::from_low_u64_be(3),
                declared_gas: 50_000,
                proposal_period: 7,
                estimate_gas_limit: 30_000,
            })?,
            TransactionServiceGasEstimationPlan::ExecuteEvm
        );

        std::fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    #[test]
    fn runtime_lookup_transaction_views_enforces_source_precedence_and_bounds() -> Result<()> {
        let temp_dir =
            unique_temp_dir("rustaxa_consensus_transaction_service_lookup_transaction_views");
        let storage = Arc::new(Storage::new(Config::new(temp_dir.clone()))?);
        let service = TransactionService::restore(
            storage,
            TransactionServiceConfig {
                queue_max_size: 16,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: ethereum_types::U256::one(),
                    history_blocks: 0,
                    is_light_node: false,
                    blocks_gas_pricer: false,
                },
                proposal_dag_gas_limit: 1_000_000,
            },
        )?;

        let mut runtime = service.lock()?;
        runtime
            .queue
            .insert(
                crate::transaction_queue::TransactionQueueEntry {
                    hash: ethereum_types::H256::from_low_u64_be(1),
                    sender: ethereum_types::H160::from_low_u64_be(11),
                    nonce: U256::zero(),
                    gas_price: U256::from(2_u64),
                    gas: 21_000,
                    data_size: 0,
                    rlp: vec![0xAA, 0xBB, 0xCC],
                    last_block_number: 0,
                },
                true,
            )
            .expect("queue insert should seed queue source");

        runtime
            .sidecar
            .insert_non_finalized(ethereum_types::H256::from_low_u64_be(2), vec![0x22])
            .expect("sidecar insert should seed non-finalized source");
        runtime
            .sidecar
            .insert_recently_finalized(7, ethereum_types::H256::from_low_u64_be(3), vec![0x33])
            .expect("sidecar insertion should seed recently-finalized source");
        runtime
            .storage
            .transaction()
            .write(ethereum_types::H256::from_low_u64_be(4), &[0x44])
            .expect("storage write should seed pending source");
        runtime
            .storage
            .transaction()
            .write_location(ethereum_types::H256::from_low_u64_be(5), 9, 0, false)
            .expect("storage location should seed excluded request");

        let period_data = period_data_with_transaction_rlps(&[vec![0x55]]);
        runtime
            .storage
            .period()
            .write(9, &period_data)
            .expect("period source should persist finalized tx");

        drop(runtime);
        let plan = service.transaction_views(
            vec![
                transaction_service_view_request(1, 1),
                transaction_service_view_request(2, 2),
                transaction_service_view_request(3, 3),
                transaction_service_view_request(4, 4),
                transaction_service_view_request(5, 5),
                transaction_service_view_request(6, 6),
            ],
            4,
        )?;

        assert_eq!(plan.requested_count, 4);
        assert!(!plan.complete);
        assert_eq!(plan.views.len(), 4);
        assert_eq!(plan.views[0].source, TM_TRANSACTION_VIEW_SOURCE_QUEUE);
        assert!(plan.views[0].found);
        assert_eq!(plan.views[0].tx_rlp, vec![0xAA, 0xBB, 0xCC]);
        assert_eq!(
            plan.views[1].source,
            TM_TRANSACTION_VIEW_SOURCE_NON_FINALIZED_SIDECAR
        );
        assert!(plan.views[1].found);
        assert_eq!(plan.views[1].tx_rlp, vec![0x22]);
        assert_eq!(
            plan.views[2].source,
            TM_TRANSACTION_VIEW_SOURCE_RECENTLY_FINALIZED_SIDECAR
        );
        assert!(plan.views[2].found);
        assert_eq!(
            plan.views[3].source,
            TM_TRANSACTION_VIEW_SOURCE_STORAGE_PENDING
        );
        assert!(plan.views[3].found);

        std::fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    #[test]
    fn runtime_lookup_proposal_transaction_views_marks_old_finalized_transactions() -> Result<()> {
        let temp_dir = unique_temp_dir(
            "rustaxa_consensus_transaction_service_lookup_proposal_transaction_old_finalized",
        );
        let storage = Arc::new(Storage::new(Config::new(temp_dir.clone()))?);
        let service = TransactionService::restore(
            storage,
            TransactionServiceConfig {
                queue_max_size: 16,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: ethereum_types::U256::one(),
                    history_blocks: 0,
                    is_light_node: false,
                    blocks_gas_pricer: false,
                },
                proposal_dag_gas_limit: 1_000_000,
            },
        )?;

        let runtime = service.lock()?;
        let signing_key = SigningKey::from_slice(&[0x33u8; 32])?;
        let transaction_rlp = signed_legacy_transaction_rlp(&signing_key);
        let transaction_hash = keccak256(&transaction_rlp);
        let envelope = LegacyTransactionEnvelope::decode(&transaction_rlp)
            .expect("proposal test transaction should decode");
        runtime
            .storage
            .transaction()
            .write_location(transaction_hash, 1, 0, false)
            .expect("proposal storage location should persist");

        let period_data = period_data_with_transaction_rlps(&[transaction_rlp.clone()]);
        runtime
            .storage
            .period()
            .write(1, &period_data)
            .expect("proposal period data should persist");

        drop(runtime);
        let plan = service.proposal_transaction_views(
            1,
            vec![TransactionServiceTransactionViewRequest {
                input_index: 10,
                hash: transaction_hash.to_fixed_bytes(),
            }],
            vec![TransactionServiceAccountNonceFact {
                sender: envelope
                    .sender
                    .expect("proposal test should expose sender")
                    .0,
                account_found: true,
                account_nonce: (envelope.nonce + U256::from(1u64)).to_big_endian(),
            }],
            0,
        )?;

        assert_eq!(plan.requested_count, 1);
        assert!(plan.complete);
        assert_eq!(plan.views.len(), 1);
        assert!(!plan.views[0].found);
        assert!(plan.views[0].old_finalized);
        assert_eq!(
            plan.views[0].source,
            TM_TRANSACTION_VIEW_SOURCE_STORAGE_FINALIZED_REGULAR
        );
        assert!(plan.views[0].tx_rlp.is_empty());

        let permissive_plan = service.proposal_transaction_views(
            1,
            vec![TransactionServiceTransactionViewRequest {
                input_index: 10,
                hash: transaction_hash.to_fixed_bytes(),
            }],
            vec![],
            0,
        )?;
        assert!(permissive_plan.views[0].found);

        let runtime = service.lock()?;
        let strict_error = runtime
            .lookup_proposal_transaction_views_requiring_account_nonce_facts(
                1,
                vec![TransactionServiceTransactionViewRequest {
                    input_index: 10,
                    hash: transaction_hash.to_fixed_bytes(),
                }],
                vec![],
                0,
            )
            .err();
        assert!(
            strict_error
                .expect("missing finalized nonce fact should be reported")
                .to_string()
                .contains("TM_PROPOSAL_FINALIZED_ACCOUNT_NONCE_FACT_MISSING")
        );

        std::fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    #[test]
    fn runtime_lookup_proposal_transaction_views_validates_stored_transaction_hash() -> Result<()> {
        let temp_dir = unique_temp_dir(
            "rustaxa_consensus_transaction_service_lookup_proposal_transaction_hash",
        );
        let storage = Arc::new(Storage::new(Config::new(temp_dir.clone()))?);
        let service = TransactionService::restore(
            storage,
            TransactionServiceConfig {
                queue_max_size: 16,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: ethereum_types::U256::one(),
                    history_blocks: 0,
                    is_light_node: false,
                    blocks_gas_pricer: false,
                },
                proposal_dag_gas_limit: 1_000_000,
            },
        )?;
        let runtime = service.lock()?;
        let corrupt_hash = H256::from_low_u64_be(8);
        let mismatched_transaction_rlp =
            signed_legacy_transaction_rlp(&SigningKey::from_slice(&[0x44u8; 32])?);

        runtime
            .storage
            .transaction()
            .write_location(corrupt_hash, 2, 0, false)
            .expect("proposal storage location should persist");
        let period_data = period_data_with_transaction_rlps(&[mismatched_transaction_rlp]);
        runtime
            .storage
            .period()
            .write(2, &period_data)
            .expect("proposal period data should persist");

        let err = runtime
            .lookup_proposal_transaction_views_with_account_nonce_facts(
                2,
                vec![TransactionServiceTransactionViewRequest {
                    input_index: 1,
                    hash: corrupt_hash.to_fixed_bytes(),
                }],
                vec![],
                0,
            )
            .err();

        assert!(
            err.expect("hash mismatch should be rejected")
                .to_string()
                .contains("TM_PROPOSAL_TRANSACTION_HASH_MISMATCH")
        );

        std::fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    #[test]
    fn native_direct_mutations_and_compatibility_pack_own_the_lock() -> Result<()> {
        let (service, temp_dir) = build_service_with_defaults(
            None,
            16,
            GasPricerConfig {
                percentile: 50,
                minimum_price: U256::one(),
                history_blocks: 0,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        let transaction_rlp =
            signed_legacy_transaction_rlp(&SigningKey::from_slice(&[0x71u8; 32])?);
        let envelope = LegacyTransactionEnvelope::decode(&transaction_rlp)?;
        let sender = envelope.sender.context("signed transaction sender")?;
        {
            let mut state = service.lock()?;
            state.queue.insert(
                TransactionQueueEntry {
                    hash: envelope.hash,
                    sender,
                    nonce: envelope.nonce,
                    gas_price: envelope.gas_price,
                    gas: envelope.gas,
                    data_size: envelope.data.len() as u64,
                    rlp: transaction_rlp.clone(),
                    last_block_number: 0,
                },
                true,
            )?;
        }

        let prepared =
            service.prepare_compatibility_pack(TransactionServiceCompatibilityPackRequest {
                weight_limit: 63_000,
                min_transaction_gas: 21_000,
                proposal_period: 7,
                estimate_gas_limit: 0,
                last_block_number: 10,
                total_shards: 1,
                node_shard: 0,
                shard_period_interval: 1,
            })?;
        assert_eq!(prepared.request_estimates.len(), 1);
        assert_eq!(prepared.request_estimates[0].hash, envelope.hash);
        assert!(prepared.selected_transactions.is_empty());

        let finalized =
            service.finalize_compatibility_pack(vec![TransactionServicePackEstimate {
                hash: envelope.hash,
                gas_used: 21_000,
                last_block_number: 10,
                result_rlp: vec![0xC0],
            }])?;
        assert_eq!(finalized.selected_transactions.len(), 1);
        assert_eq!(finalized.selected_transactions[0].hash, envelope.hash);
        assert!(!service.abort_compatibility_pack()?);

        let cached_hash = H256::from_low_u64_be(9);
        assert!(
            service.store_gas_estimation(TransactionServiceGasEstimationResult {
                hash: cached_hash,
                proposal_period: 7,
                gas_used: 19_000,
                result_rlp: vec![0xC1],
            },)?
        );
        assert_eq!(
            service.plan_gas_estimation(TransactionServiceGasEstimationRequest {
                hash: cached_hash,
                declared_gas: 50_000,
                proposal_period: 7,
                estimate_gas_limit: 30_000,
            })?,
            TransactionServiceGasEstimationPlan::Cached {
                gas_used: 19_000,
                result_rlp: vec![0xC1],
            }
        );

        let recent_hash = H256::from_low_u64_be(10);
        service.initialize_recently_finalized(
            8,
            vec![TransactionServicePayload {
                hash: recent_hash,
                transaction_rlp: vec![0x22],
            }],
        )?;
        assert!(service.is_transaction_known(recent_hash.0)?);

        let removable_hash = H256::from_low_u64_be(11);
        {
            let mut state = service.lock()?;
            state.storage.transaction().write(removable_hash, &[0x33])?;
            state
                .sidecar
                .insert_non_finalized(removable_hash, vec![0x33])?;
        }
        assert_eq!(service.remove_non_finalized(vec![removable_hash])?, 1);
        assert_eq!(
            service.lock()?.storage.transaction().rlp(removable_hash)?,
            None
        );

        let expired_hash = H256::from_low_u64_be(12);
        service.lock()?.queue.insert(
            TransactionQueueEntry {
                hash: expired_hash,
                sender,
                nonce: U256::from(2),
                gas_price: U256::one(),
                gas: 21_000,
                data_size: 0,
                rlp: vec![0x44],
                last_block_number: 0,
            },
            false,
        )?;
        assert_eq!(service.queue_block_finalized(20)?, vec![expired_hash]);

        service.update_gas_prices(vec![U256::from(5)])?;

        let finalized_hash = H256::from_low_u64_be(13);
        service
            .lock()?
            .storage
            .transaction()
            .write_location(finalized_hash, 9, 0, false)?;
        let filtered = service.filter_non_finalized(vec![
            TransactionServiceFinalizedFilterRequest {
                input_index: 0,
                hash: H256::from_low_u64_be(14),
            },
            TransactionServiceFinalizedFilterRequest {
                input_index: 1,
                hash: finalized_hash,
            },
            TransactionServiceFinalizedFilterRequest {
                input_index: 2,
                hash: recent_hash,
            },
        ])?;
        assert_eq!(filtered.not_finalized.len(), 1);
        assert_eq!(filtered.not_finalized[0].input_index, 0);

        let recent_outcome =
            service.verify_not_finalized(vec![TransactionServiceVerifyNotFinalizedFact {
                input_index: 4,
                hash: recent_hash,
                transaction_nonce: U256::MAX,
                sender_account_nonce: U256::zero(),
            }])?;
        assert_eq!(
            recent_outcome.source,
            TM_VERIFY_NOT_FINALIZED_SOURCE_RECENT_SIDECAR
        );
        let storage_outcome =
            service.verify_not_finalized(vec![TransactionServiceVerifyNotFinalizedFact {
                input_index: 5,
                hash: finalized_hash,
                transaction_nonce: U256::one(),
                sender_account_nonce: U256::one(),
            }])?;
        assert_eq!(
            storage_outcome.source,
            TM_VERIFY_NOT_FINALIZED_SOURCE_STORAGE
        );

        let recovery_rlp = signed_legacy_transaction_rlp(&SigningKey::from_slice(&[0x73u8; 32])?);
        let recovery_envelope = LegacyTransactionEnvelope::decode(&recovery_rlp)?;
        service
            .lock()?
            .storage
            .transaction()
            .write(recovery_envelope.hash, &recovery_rlp)?;
        assert_eq!(service.recover_non_finalized()?, 1);
        assert!(
            service
                .non_finalized_transaction_views(vec![TransactionServiceTransactionViewRequest {
                    input_index: 0,
                    hash: recovery_envelope.hash.0,
                },])?
                .first()
                .is_some_and(|view| view.found)
        );

        std::fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    #[test]
    fn compatibility_pack_acknowledges_prepare_demotions_on_finalize() -> Result<()> {
        let (service, temp_dir) = build_service_with_defaults(
            None,
            16,
            GasPricerConfig {
                percentile: 50,
                minimum_price: U256::one(),
                history_blocks: 0,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        let signing_key = SigningKey::from_slice(&[0x53u8; 32])?;
        let mut envelopes = Vec::new();
        {
            let mut state = service.lock()?;
            for nonce in [U256::one(), U256::from(2)] {
                let rlp = signed_legacy_transaction_rlp_with_fields(
                    &signing_key,
                    nonce,
                    U256::from(2),
                    21_000,
                    U256::from(3),
                );
                let envelope = LegacyTransactionEnvelope::decode(&rlp)?;
                state.queue.insert(
                    TransactionQueueEntry {
                        hash: envelope.hash,
                        sender: envelope.sender.context("signed transaction sender")?,
                        nonce: envelope.nonce,
                        gas_price: envelope.gas_price,
                        gas: envelope.gas,
                        data_size: envelope.data.len() as u64,
                        rlp,
                        last_block_number: 0,
                    },
                    true,
                )?;
                envelopes.push(envelope);
            }
        }
        service.store_gas_estimation(TransactionServiceGasEstimationResult {
            hash: envelopes[0].hash,
            proposal_period: 7,
            gas_used: 20_000,
            result_rlp: vec![0xC0],
        })?;

        let prepared =
            service.prepare_compatibility_pack(TransactionServiceCompatibilityPackRequest {
                weight_limit: 63_000,
                min_transaction_gas: 21_000,
                proposal_period: 7,
                estimate_gas_limit: 0,
                last_block_number: 44,
                total_shards: 1,
                node_shard: 0,
                shard_period_interval: 1,
            })?;
        assert_eq!(prepared.request_estimates.len(), 1);
        assert_eq!(prepared.request_estimates[0].hash, envelopes[1].hash);
        assert_eq!(prepared.demoted_hashes, vec![envelopes[0].hash]);
        assert_eq!(service.queue_size()?, 1);

        let finalized =
            service.finalize_compatibility_pack(vec![TransactionServicePackEstimate {
                hash: envelopes[1].hash,
                gas_used: 30_000,
                last_block_number: 44,
                result_rlp: vec![0xC1],
            }])?;
        assert_eq!(finalized.demoted_hashes, vec![envelopes[0].hash]);
        assert_eq!(finalized.selected_transactions[0].hash, envelopes[1].hash);

        std::fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    #[test]
    fn compatibility_pack_rejects_overflowing_envelope_cost_and_aborts() -> Result<()> {
        let (service, temp_dir) = build_service_with_defaults(
            None,
            16,
            GasPricerConfig {
                percentile: 50,
                minimum_price: U256::one(),
                history_blocks: 0,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        let rlp = signed_legacy_transaction_rlp_with_fields(
            &SigningKey::from_slice(&[0x72u8; 32])?,
            U256::one(),
            U256::MAX,
            21_000,
            U256::from(3),
        );
        let envelope = LegacyTransactionEnvelope::decode(&rlp)?;
        service.lock()?.queue.insert(
            TransactionQueueEntry {
                hash: envelope.hash,
                sender: envelope.sender.context("signed transaction sender")?,
                nonce: envelope.nonce,
                gas_price: envelope.gas_price,
                gas: envelope.gas,
                data_size: envelope.data.len() as u64,
                rlp,
                last_block_number: 0,
            },
            true,
        )?;

        let error = service
            .prepare_compatibility_pack(TransactionServiceCompatibilityPackRequest {
                weight_limit: 63_000,
                min_transaction_gas: 21_000,
                proposal_period: 7,
                estimate_gas_limit: 0,
                last_block_number: 44,
                total_shards: 1,
                node_shard: 0,
                shard_period_interval: 1,
            })
            .expect_err("overflowing envelope cost must fail packing");
        assert!(
            error
                .to_string()
                .contains("TM_RUNTIME_PACK_CANDIDATE_ENVELOPE_INSPECT_FAILED")
        );
        assert!(!service.abort_compatibility_pack()?);

        std::fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    #[test]
    fn recovery_validation_failure_does_not_publish_partial_sidecar_state() -> Result<()> {
        let (service, temp_dir) = build_service_with_defaults(
            None,
            16,
            GasPricerConfig {
                percentile: 50,
                minimum_price: U256::one(),
                history_blocks: 0,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        let existing_hash = H256::from_low_u64_be(1);
        service
            .lock()?
            .sidecar
            .insert_non_finalized(existing_hash, vec![0x11])?;

        let valid_rlp = signed_legacy_transaction_rlp(&SigningKey::from_slice(&[0x74u8; 32])?);
        let valid = LegacyTransactionEnvelope::decode(&valid_rlp)?;
        let (overflow_rlp, overflow) = (0x75u8..=0xFE)
            .find_map(|key_byte| {
                let signing_key = SigningKey::from_slice(&[key_byte; 32]).ok()?;
                let rlp = signed_legacy_transaction_rlp_with_fields(
                    &signing_key,
                    U256::one(),
                    U256::MAX,
                    21_000,
                    U256::from(3),
                );
                let envelope = LegacyTransactionEnvelope::decode(&rlp).ok()?;
                (envelope.hash > valid.hash).then_some((rlp, envelope))
            })
            .context("test must find an overflowing transaction ordered after survivor")?;
        {
            let state = service.lock()?;
            state.storage.transaction().write(valid.hash, &valid_rlp)?;
            state
                .storage
                .transaction()
                .write(overflow.hash, &overflow_rlp)?;
        }

        let error = service
            .recover_non_finalized()
            .expect_err("overflowing recovery envelope must fail");
        assert!(
            error
                .to_string()
                .contains("TM_NONFINALIZED_RECOVERY_ENVELOPE_INSPECT")
        );
        let state = service.lock()?;
        assert!(state.sidecar.contains_non_finalized(existing_hash));
        assert!(!state.sidecar.contains_non_finalized(valid.hash));
        drop(state);

        std::fs::remove_dir_all(temp_dir)?;
        Ok(())
    }

    #[test]
    fn native_admission_owns_mutation_and_public_error_precedence() -> Result<()> {
        let (service, path) = build_service_with_defaults(
            None,
            8,
            GasPricerConfig {
                percentile: 50,
                minimum_price: U256::one(),
                history_blocks: 0,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        let hash = H256::from_low_u64_be(0xa1);
        let entry = TransactionQueueEntry {
            hash,
            sender: H160::from_low_u64_be(0xb1),
            nonce: U256::from(3),
            gas_price: U256::from(4),
            gas: 21_000,
            data_size: 0,
            rlp: vec![0xc0],
            last_block_number: 9,
        };
        let admission_fact = TransactionServiceValidatedAdmissionFact {
            tx_hash: hash,
            transaction_nonce: U256::from(3),
            transaction_cost: U256::from(5),
            gas_limit: 21_000,
            proposal_dag_gas_limit: 30_000,
            insert_non_proposable: false,
        };
        let final_chain_fact = TransactionServiceFinalChainAdmissionFact {
            account_found: true,
            account_nonce: U256::from(3),
            account_balance: U256::from(100),
            finalized_period: None,
        };

        let accepted =
            service.execute_admission(admission_fact, final_chain_fact, entry.clone())?;
        assert_eq!(
            accepted.insert_status,
            TransactionManagerInsertTransactionStatus::Accepted
        );
        assert_eq!(accepted.inserted_hash, Some(hash));
        assert!(accepted.emit_transaction_added);

        let known = service.execute_public_admission(
            TransactionManagerVerifyTransactionFact {
                tx_hash: hash,
                chain_id: 1,
                expected_chain_id: 2,
                gas_limit: u64::MAX,
                max_gas_limit: 0,
                last_block_number: 0,
                cornus_active: true,
                intrinsic_gas_covered: false,
                signature_valid: false,
                gas_price: U256::zero(),
                minimum_gas_price: U256::one(),
            },
            admission_fact,
            final_chain_fact,
            entry,
        )?;
        assert_eq!(
            known.verification_status,
            TransactionManagerVerifyTransactionStatus::Accepted
        );
        assert_eq!(
            known
                .admission
                .as_ref()
                .expect("known precheck returns admission")
                .insert_status,
            TransactionManagerInsertTransactionStatus::AlreadyKnown
        );
        assert_eq!(
            known
                .admission
                .expect("known precheck returns admission")
                .transaction_status,
            TransactionQueueInsertStatus::Known
        );
        assert_eq!(
            known.public_result.message,
            "Transaction already in transactions pool"
        );

        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn native_admission_records_queue_overflow_drop_window() -> Result<()> {
        let (service, path) = build_service_with_defaults(
            None,
            1,
            GasPricerConfig {
                percentile: 50,
                minimum_price: U256::one(),
                history_blocks: 0,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        assert!(!service.queue_transactions_dropped()?);

        for identity in 1_u64..=2 {
            let hash = H256::from_low_u64_be(identity);
            service.execute_admission(
                TransactionServiceValidatedAdmissionFact {
                    tx_hash: hash,
                    transaction_nonce: U256::zero(),
                    transaction_cost: U256::one(),
                    gas_limit: 21_000,
                    proposal_dag_gas_limit: 30_000,
                    insert_non_proposable: false,
                },
                TransactionServiceFinalChainAdmissionFact {
                    account_found: true,
                    account_nonce: U256::zero(),
                    account_balance: U256::from(100),
                    finalized_period: None,
                },
                TransactionQueueEntry {
                    hash,
                    sender: H160::from_low_u64_be(identity),
                    nonce: U256::zero(),
                    gas_price: U256::from(identity + 1),
                    gas: 21_000,
                    data_size: 0,
                    rlp: vec![0xc0],
                    last_block_number: 0,
                },
            )?;
        }

        assert_eq!(service.queue_size()?, 1);
        assert!(service.queue_transactions_dropped()?);

        drop(service);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn public_verification_rejection_does_not_mutate_native_queue() -> Result<()> {
        let (service, path) = build_service_with_defaults(
            None,
            8,
            GasPricerConfig {
                percentile: 50,
                minimum_price: U256::one(),
                history_blocks: 0,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        let hash = H256::from_low_u64_be(0xa2);
        let entry = TransactionQueueEntry {
            hash,
            sender: H160::from_low_u64_be(0xb2),
            nonce: U256::zero(),
            gas_price: U256::one(),
            gas: 21_000,
            data_size: 0,
            rlp: vec![0xc0],
            last_block_number: 0,
        };
        let report = service.execute_public_admission(
            TransactionManagerVerifyTransactionFact {
                tx_hash: hash,
                chain_id: 1,
                expected_chain_id: 2,
                gas_limit: 21_000,
                max_gas_limit: 30_000,
                last_block_number: 0,
                cornus_active: false,
                intrinsic_gas_covered: true,
                signature_valid: true,
                gas_price: U256::one(),
                minimum_gas_price: U256::one(),
            },
            TransactionServiceValidatedAdmissionFact {
                tx_hash: hash,
                transaction_nonce: U256::zero(),
                transaction_cost: U256::one(),
                gas_limit: 21_000,
                proposal_dag_gas_limit: 30_000,
                insert_non_proposable: false,
            },
            TransactionServiceFinalChainAdmissionFact {
                account_found: true,
                account_nonce: U256::zero(),
                account_balance: U256::from(100),
                finalized_period: None,
            },
            entry,
        )?;
        assert_eq!(
            report.verification_status,
            TransactionManagerVerifyTransactionStatus::ChainIdMismatch
        );
        assert!(report.admission.is_none());
        assert_eq!(report.public_result.message, "chain_id mismatch 1 2");
        assert!(!service.is_transaction_known(hash.0)?);

        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn finalized_status_persists_before_native_sidecar_and_queue_publication() -> Result<()> {
        let (service, path) = build_service_with_defaults(
            Some(7),
            8,
            GasPricerConfig {
                percentile: 50,
                minimum_price: U256::one(),
                history_blocks: 0,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        let hash = H256::from_low_u64_be(0xf1);
        {
            let mut state = service.lock()?;
            state.sidecar.insert_non_finalized(hash, vec![0x11])?;
            state.queue.insert(
                TransactionQueueEntry {
                    hash,
                    sender: H160::from_low_u64_be(9),
                    nonce: U256::one(),
                    gas_price: U256::one(),
                    gas: 21_000,
                    data_size: 0,
                    rlp: vec![0x11],
                    last_block_number: 0,
                },
                true,
            )?;
        }

        let report = service.update_finalized_status(
            11,
            10,
            Vec::new(),
            vec![TransactionServiceFinalizedStatusFact {
                input_index: 0,
                hash,
                tx_rlp: vec![0x11],
            }],
        )?;
        assert_eq!(report.removed_non_finalized, vec![hash]);
        assert_eq!(report.queue_erased, vec![hash]);
        assert!(report.finalized_account_purged.is_empty());
        assert_eq!(report.accepted_count, 1);
        let state = service.lock()?;
        assert!(state.sidecar.contains_recently_finalized(hash));
        assert!(!state.queue.contains(hash));
        assert_eq!(state.sidecar.transaction_count(), 7);
        assert_eq!(
            state
                .storage
                .metadata()
                .status_field(StatusField::TrxCount as u8)?,
            7
        );
        drop(state);

        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn finalized_status_decodes_canonical_period_transactions_in_order() -> Result<()> {
        let first = signed_legacy_transaction_rlp(&SigningKey::from_slice(&[0x61; 32])?);
        let second = signed_legacy_transaction_rlp(&SigningKey::from_slice(&[0x62; 32])?);
        let period_data = period_data_with_transaction_rlps(&[first.clone(), second.clone()]);

        let facts = finalized_status_facts_from_period_data(&period_data)?;
        let transaction_list = Rlp::new(&period_data)
            .at(TRANSACTIONS_POS_IN_PERIOD_DATA)?
            .as_raw()
            .to_vec();
        let compatibility_facts =
            finalized_status_facts_from_transaction_list_rlp(&transaction_list)?;

        assert_eq!(
            facts,
            vec![
                TransactionServiceFinalizedStatusFact {
                    input_index: 0,
                    hash: keccak256(&first),
                    tx_rlp: first,
                },
                TransactionServiceFinalizedStatusFact {
                    input_index: 1,
                    hash: keccak256(&second),
                    tx_rlp: second,
                },
            ]
        );
        assert_eq!(compatibility_facts, facts);
        Ok(())
    }

    #[test]
    fn finalized_status_rejects_malformed_period_before_count_persistence() -> Result<()> {
        let (service, path) = build_service_with_defaults(
            Some(7),
            8,
            GasPricerConfig {
                percentile: 50,
                minimum_price: U256::one(),
                history_blocks: 0,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;

        let error = service
            .update_finalized_status_from_period_data(11, 10, Vec::new(), &[0xc0])
            .expect_err("malformed period data must reject before mutation");

        assert!(
            error
                .to_string()
                .contains("TM_FINALIZED_STATUS_PERIOD_DATA_TRANSACTIONS")
        );
        let state = service.lock()?;
        assert_eq!(state.sidecar.transaction_count(), 7);
        assert_eq!(
            state
                .storage
                .metadata()
                .status_field(StatusField::TrxCount as u8)?,
            7
        );
        drop(state);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn finalized_status_rejects_bad_input_index_before_count_persistence() -> Result<()> {
        let (service, path) = build_service_with_defaults(
            Some(7),
            8,
            GasPricerConfig {
                percentile: 50,
                minimum_price: U256::one(),
                history_blocks: 0,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        let error = service
            .update_finalized_status(
                11,
                10,
                Vec::new(),
                vec![TransactionServiceFinalizedStatusFact {
                    input_index: 9,
                    hash: H256::from_low_u64_be(0xf2),
                    tx_rlp: vec![0x12],
                }],
            )
            .expect_err("bad input index must fail before persistence");
        assert!(
            error
                .to_string()
                .contains("TM_RUNTIME_FINALIZED_STATUS_INPUT_INDEX")
        );
        let state = service.lock()?;
        assert_eq!(state.sidecar.transaction_count(), 7);
        assert_eq!(
            state
                .storage
                .metadata()
                .status_field(StatusField::TrxCount as u8)?,
            7
        );
        drop(state);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn invalid_gas_config_fails_before_service_publication() -> Result<()> {
        let path = unique_temp_dir("rustaxa_consensus_transaction_service_invalid_config");
        let storage = Arc::new(Storage::new(Config::new(path.clone()))?);
        let error = TransactionService::restore(
            storage,
            TransactionServiceConfig {
                queue_max_size: 8,
                gas_pricer_config: GasPricerConfig {
                    percentile: 101,
                    minimum_price: ethereum_types::U256::one(),
                    history_blocks: 1,
                    is_light_node: false,
                    blocks_gas_pricer: true,
                },
                proposal_dag_gas_limit: 42_000,
            },
        )
        .err()
        .expect("invalid percentile must reject construction");
        assert!(error.to_string().contains("percentile"));
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[test]
    fn poisoned_service_lock_uses_stable_identifier() -> Result<()> {
        let (service, path) = build_service_with_defaults(
            None,
            8,
            GasPricerConfig {
                percentile: 50,
                minimum_price: ethereum_types::U256::one(),
                history_blocks: 0,
                is_light_node: false,
                blocks_gas_pricer: false,
            },
        )?;
        std::thread::scope(|scope| {
            let handle = scope.spawn(|| {
                let _guard = service.state.lock().unwrap();
                panic!("poison native transaction service");
            });
            assert!(handle.join().is_err());
        });
        assert_eq!(
            service
                .lock()
                .err()
                .expect("poisoned service must reject locking")
                .to_string(),
            DAG_TRANSACTION_SERVICE_TRANSACTION_LOCK_POISONED
        );
        std::fs::remove_dir_all(path)?;
        Ok(())
    }
}
