//! CXX bridge wrappers for Rust `TransactionManager` decision boundaries.
//!
//! The bridge exposes:
//! - a short-lived planner used while one DAG proposal is being packed
//! - a storage-complete planner for `TransactionManager::saveTransactionsFromDagBlock`
//! - an opaque runtime handle that owns non-finalized and recently-finalized payload sidecars
//!
//! C++ supplies transaction metadata, RLP payloads, and queue-known facts. Rust owns
//! deterministic planning, latest-state FinalChain account fact sourcing, storage
//! mutations routed through Rust storage, live transaction count authority, admission
//! status mapping, and sidecar membership/RLP bytes, but not C++ `Transaction`
//! pointers or gas estimation.

#[cfg(test)]
use crate::ffi::rustaxa_ffi::TransactionManagerSidecarInsertInput;
#[cfg(test)]
use crate::ffi::rustaxa_ffi::TransactionQueueConfig;
use crate::ffi::rustaxa_ffi::{
    DagTransactionSaveSidecarFact, FinalizedTransactionStatusSidecarFact, GasPricerConfig,
    TransactionManagerAdmissionCommandReport, TransactionManagerAdmissionResult,
    TransactionManagerAdmissionShellIntent, TransactionManagerDagSaveCommandReport,
    TransactionManagerFinalChainAdmissionFact, TransactionManagerFinalizedStatusCommandReport,
    TransactionManagerGasEstimationFact, TransactionManagerGasEstimationPlan,
    TransactionManagerHashCommand, TransactionManagerPublicAdmissionCommandReport,
    TransactionManagerPublicInsertResult, TransactionManagerTransactionView,
    TransactionManagerTransactionViewPlan, TransactionManagerTransactionViewRequest,
    TransactionManagerValidatedInsertRuntimeFact, TransactionManagerVerifyTransactionFact,
    TransactionManagerVerifyTransactionOutcome,
    TransactionQueueAccountNonceFact as BridgeTransactionQueueAccountNonceFact,
    TransactionQueueHash, TransactionQueueInsertInput, TransactionQueueStoredTransaction,
    TransactionQueueTransactionGroup,
};
use anyhow::{ensure, Context, Result};
use ethereum_types::{H160, H256, U256};
use rustaxa_consensus::gas_pricer::GasPricerConfig as DomainGasPricerConfig;
use rustaxa_consensus::transaction_manager::{
    plan_finalized_transactions_status, plan_insert_transaction, plan_transactions_from_dag_block,
    plan_validated_insert, plan_verify_transaction,
    DagTransactionSaveFact as ConsensusDagTransactionSaveFact,
    FinalizedTransactionStatusFact as ConsensusFinalizedTransactionStatusFact,
    FinalizedTransactionStatusPlan as ConsensusFinalizedTransactionStatusPlan,
    TransactionManagerInsertTransactionFact as ConsensusTransactionManagerInsertTransactionFact,
    TransactionManagerInsertTransactionStatus, TransactionManagerKnownFact,
    TransactionManagerValidatedInsertFact as ConsensusTransactionManagerValidatedInsertFact,
    TransactionManagerVerifyTransactionFact as ConsensusTransactionManagerVerifyTransactionFact,
    TransactionManagerVerifyTransactionStatus,
};
use rustaxa_consensus::transaction_queue::{
    TransactionQueue, TransactionQueueAccountNonceFact, TransactionQueueEntry,
    TransactionQueueInsertStatus, TransactionQueuePurgeOutcome,
};
#[cfg(test)]
use rustaxa_consensus::transaction_service::TransactionServiceConfig;
use rustaxa_consensus::transaction_service::{
    TransactionServiceAccountNonceFact, TransactionServiceGasEstimationPlan,
    TransactionServiceGasEstimationRequest, TransactionServiceGuard, TransactionServiceState,
    TransactionServiceTransactionView, TransactionServiceTransactionViewPlan,
    TransactionServiceTransactionViewRequest,
};
use rustaxa_consensus::transaction_storage::{
    append_non_finalized_transactions_to_batch, save_transaction_count, transaction_finalized,
    NonFinalizedTransactionStoragePayload,
};
#[cfg(test)]
use rustaxa_storage::StatusField;
use rustaxa_storage::{Storage, StorageWriteBatch};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

struct TransactionManagerRuntimeQueueCleanupPlan {
    non_proposable_expired: TransactionManagerRuntimeQueuePurgePlan,
    finalized_account_purged: TransactionManagerRuntimeQueuePurgePlan,
}

/// Prepared DAG transaction persistence held until a shared DAG/transaction
/// storage batch commits.
pub(crate) struct PreparedDagTransactionSave {
    accepted: Vec<DagTransactionSaveAccepted>,
    accepted_payloads: Vec<NonFinalizedTransactionStoragePayload>,
    target_transaction_count: u64,
}

/// Fully prevalidated live-state publication for a committed DAG transaction save.
pub(crate) struct PreparedDagTransactionPublication {
    queue: TransactionQueue,
    sidecar: rustaxa_consensus::transaction_manager::TransactionManagerSidecar,
    pub(crate) outcome: DagTransactionSaveOutcome,
}

/// Temporary FFI adapter over native transaction state.
///
/// Production wraps a short-lived [`TransactionServiceGuard`]; focused bridge
/// tests wrap an owned state. Both paths keep FFI-shaped behavior in this crate
/// without returning or storing a native guard across CXX.
pub(crate) struct TransactionRuntimeAccess<T>(pub(crate) T);

impl<T> Deref for TransactionRuntimeAccess<T>
where
    T: Deref<Target = TransactionServiceState>,
{
    type Target = TransactionServiceState;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for TransactionRuntimeAccess<T>
where
    T: DerefMut<Target = TransactionServiceState>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub(crate) type TransactionRuntimeGuard<'a> = TransactionRuntimeAccess<TransactionServiceGuard<'a>>;
#[cfg(test)]
type TransactionRuntimeState = TransactionRuntimeAccess<TestTransactionServiceState>;

/// Owned transaction state used by bridge tests.
///
/// Tests that create their own RocksDB directory attach it as `cleanup_path`.
/// Dropping the fixture closes storage before removing that directory, while
/// tests borrowing an externally managed storage owner leave cleanup to it.
#[cfg(test)]
struct TestTransactionServiceState {
    state: Option<Box<TransactionServiceState>>,
    cleanup_path: Option<std::path::PathBuf>,
}

#[cfg(test)]
impl Deref for TestTransactionServiceState {
    type Target = TransactionServiceState;

    fn deref(&self) -> &Self::Target {
        self.state
            .as_deref()
            .expect("test transaction state should remain available")
    }
}

#[cfg(test)]
impl DerefMut for TestTransactionServiceState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.state
            .as_deref_mut()
            .expect("test transaction state should remain available")
    }
}

#[cfg(test)]
impl Drop for TestTransactionServiceState {
    fn drop(&mut self) {
        drop(self.state.take());
        if let Some(path) = self.cleanup_path.take() {
            std::fs::remove_dir_all(&path).unwrap_or_else(|error| {
                panic!(
                    "test transaction storage cleanup failed for {}: {error}",
                    path.display()
                )
            });
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct TransactionManagerRuntimeQueueInsertOutcome {
    status: u8,
    inserted_hash_found: bool,
    inserted_hash: [u8; 32],
    demoted_hashes: Vec<TransactionQueueHash>,
    overflow_removed_hashes: Vec<TransactionQueueHash>,
}

struct TransactionManagerRuntimeQueuePurgePlan {
    removed_hashes: Vec<TransactionQueueHash>,
}

/// Module-private result of one fact-backed runtime admission command.
///
/// Inputs are the validated-insert plan, queue mutation result, and FinalChain
/// facts assembled by the runtime. The carrier preserves the public insertion
/// statuses, finalized-period evidence, and live shell-effect hash until it is
/// converted into `TransactionManagerAdmissionCommandReport`. It never crosses
/// CXX, owns no queue entries, and represents no fallible state: errors are
/// returned before construction.
struct TransactionManagerRuntimeAdmissionOutcome {
    insert_status: u8,
    transaction_status: u8,
    requires_finalized_lookup: bool,
    finalized_period_known: bool,
    finalized_period: u64,
    emit_transaction_added: bool,
    inserted_hash_found: bool,
    inserted_hash: [u8; 32],
}

struct DagTransactionSaveAccepted {
    hash: [u8; 32],
    erased_from_queue: bool,
}

fn bridge_to_service_transaction_view_request(
    request: TransactionManagerTransactionViewRequest,
) -> TransactionServiceTransactionViewRequest {
    TransactionServiceTransactionViewRequest {
        input_index: request.input_index,
        hash: request.hash,
    }
}

pub(crate) fn bridge_to_service_transaction_view_requests(
    requests: Vec<TransactionManagerTransactionViewRequest>,
) -> Vec<TransactionServiceTransactionViewRequest> {
    requests
        .into_iter()
        .map(bridge_to_service_transaction_view_request)
        .collect()
}

pub(crate) fn service_to_bridge_transaction_view(
    view: TransactionServiceTransactionView,
) -> TransactionManagerTransactionView {
    TransactionManagerTransactionView {
        input_index: view.input_index,
        hash: view.hash,
        found: view.found,
        source: view.source,
        old_finalized: view.old_finalized,
        tx_rlp: view.tx_rlp,
    }
}

pub(crate) fn service_to_bridge_transaction_view_plan(
    plan: TransactionServiceTransactionViewPlan,
) -> TransactionManagerTransactionViewPlan {
    TransactionManagerTransactionViewPlan {
        requested_count: plan.requested_count,
        complete: plan.complete,
        views: plan
            .views
            .into_iter()
            .map(service_to_bridge_transaction_view)
            .collect(),
    }
}

pub(crate) fn bridge_to_service_account_nonce_facts(
    account_nonce_facts: Vec<BridgeTransactionQueueAccountNonceFact>,
) -> Vec<TransactionServiceAccountNonceFact> {
    account_nonce_facts
        .into_iter()
        .map(|fact| TransactionServiceAccountNonceFact {
            sender: fact.sender,
            account_found: fact.account_found,
            account_nonce: fact.account_nonce,
        })
        .collect()
}

pub(crate) fn bridge_to_service_gas_estimation_request(
    fact: TransactionManagerGasEstimationFact,
) -> TransactionServiceGasEstimationRequest {
    TransactionServiceGasEstimationRequest {
        hash: H256::from(fact.hash),
        declared_gas: fact.declared_gas,
        proposal_period: fact.proposal_period,
        estimate_gas_limit: fact.estimate_gas_limit,
    }
}

pub(crate) fn service_to_bridge_gas_estimation_plan(
    plan: TransactionServiceGasEstimationPlan,
) -> TransactionManagerGasEstimationPlan {
    match plan {
        TransactionServiceGasEstimationPlan::Declared { gas_used } => {
            TransactionManagerGasEstimationPlan {
                use_declared_gas: true,
                cache_hit: false,
                requires_evm_call: false,
                gas_used,
                result_rlp: Vec::new(),
            }
        }
        TransactionServiceGasEstimationPlan::Cached {
            gas_used,
            result_rlp,
        } => TransactionManagerGasEstimationPlan {
            use_declared_gas: false,
            cache_hit: true,
            requires_evm_call: false,
            gas_used,
            result_rlp,
        },
        TransactionServiceGasEstimationPlan::ExecuteEvm => TransactionManagerGasEstimationPlan {
            use_declared_gas: false,
            cache_hit: false,
            requires_evm_call: true,
            gas_used: 0,
            result_rlp: Vec::new(),
        },
    }
}

pub(crate) struct DagTransactionSaveOutcome {
    accepted: Vec<DagTransactionSaveAccepted>,
}

struct FinalizedTransactionStatusAction {
    hash: [u8; 32],
    removed_non_finalized: bool,
    erase_from_queue: bool,
    erased_from_queue: bool,
}

struct FinalizedTransactionStatusPlan {
    accepted: Vec<FinalizedTransactionStatusAction>,
    purge_transaction_queue: bool,
}

const TM_VERIFY_TRANSACTION_STATUS_ACCEPTED: u8 =
    TransactionManagerVerifyTransactionStatus::Accepted as u8;
const TM_VERIFY_TRANSACTION_STATUS_CHAIN_ID_MISMATCH: u8 =
    TransactionManagerVerifyTransactionStatus::ChainIdMismatch as u8;
const TM_VERIFY_TRANSACTION_STATUS_INVALID_GAS: u8 =
    TransactionManagerVerifyTransactionStatus::InvalidGas as u8;
const TM_VERIFY_TRANSACTION_STATUS_INTRINSIC_GAS: u8 =
    TransactionManagerVerifyTransactionStatus::IntrinsicGasNotCovered as u8;
const TM_VERIFY_TRANSACTION_STATUS_INVALID_SIGNATURE: u8 =
    TransactionManagerVerifyTransactionStatus::InvalidSignature as u8;
const TM_VERIFY_TRANSACTION_STATUS_GAS_PRICE: u8 =
    TransactionManagerVerifyTransactionStatus::GasPriceTooLow as u8;

const TM_INSERT_TRANSACTION_STATUS_ACCEPTED: u8 =
    TransactionManagerInsertTransactionStatus::Accepted as u8;
const TM_INSERT_TRANSACTION_STATUS_ALREADY_KNOWN: u8 =
    TransactionManagerInsertTransactionStatus::AlreadyKnown as u8;
const TM_INSERT_TRANSACTION_STATUS_ALREADY_FINALIZED: u8 =
    TransactionManagerInsertTransactionStatus::AlreadyFinalized as u8;
const TM_INSERT_TRANSACTION_STATUS_CANNOT_INSERT: u8 =
    TransactionManagerInsertTransactionStatus::CouldNotInsert as u8;
const TM_ADMISSION_SHELL_INTENT_LOG_INSERTED: u8 = 1;
const TM_ADMISSION_SHELL_INTENT_EMIT_TRANSACTION_ADDED: u8 = 2;
#[cfg(test)]
const TRANSACTION_QUEUE_DROP_WINDOW: Duration = Duration::from_secs(600);

/// Bridge-private facts for TransactionManager insertion planning.
///
/// These are assembled by Rust runtime admission commands from queue and
/// FinalChain facts that C++ already supplied through higher-level command
/// APIs. They intentionally stay out of the CXX surface; external callers see
/// only command reports.
struct TransactionManagerInsertTransactionFact {
    tx_hash: [u8; 32],
    hash_known: bool,
    queue_status: u8,
    has_finalized_period: bool,
    finalized_period: u64,
}

/// Bridge-private insertion planning result.
///
/// The result feeds public admission command reports inside this module and is
/// not a standalone CXX DTO.
struct TransactionManagerInsertTransactionOutcome {
    status: u8,
    finalized_period_known: bool,
    finalized_period: u64,
}

fn hash_command(hash: [u8; 32]) -> TransactionManagerHashCommand {
    TransactionManagerHashCommand { hash }
}

fn command_admission_result(
    outcome: &TransactionManagerRuntimeAdmissionOutcome,
) -> TransactionManagerAdmissionResult {
    TransactionManagerAdmissionResult {
        present: true,
        insert_status: outcome.insert_status,
        transaction_status: outcome.transaction_status,
        finalized_period_known: outcome.finalized_period_known,
        finalized_period: outcome.finalized_period,
        requires_finalized_lookup: outcome.requires_finalized_lookup,
    }
}

fn command_admission_result_from_insert_outcome(
    outcome: &TransactionManagerInsertTransactionOutcome,
) -> TransactionManagerAdmissionResult {
    TransactionManagerAdmissionResult {
        present: true,
        insert_status: outcome.status,
        transaction_status: 0,
        finalized_period_known: outcome.finalized_period_known,
        finalized_period: outcome.finalized_period,
        requires_finalized_lookup: false,
    }
}

pub(crate) fn dag_save_command_report(
    outcome: &DagTransactionSaveOutcome,
) -> TransactionManagerDagSaveCommandReport {
    let mut queue_erased = Vec::new();
    for entry in &outcome.accepted {
        if entry.erased_from_queue {
            queue_erased.push(hash_command(entry.hash));
        }
    }
    TransactionManagerDagSaveCommandReport { queue_erased }
}

fn admission_command_report(
    outcome: &TransactionManagerRuntimeAdmissionOutcome,
) -> TransactionManagerAdmissionCommandReport {
    let mut shell_intents = Vec::new();
    if outcome.inserted_hash_found {
        shell_intents.push(TransactionManagerAdmissionShellIntent {
            kind: TM_ADMISSION_SHELL_INTENT_LOG_INSERTED,
            hash: outcome.inserted_hash,
        });
    }
    if outcome.emit_transaction_added && outcome.inserted_hash_found {
        shell_intents.push(TransactionManagerAdmissionShellIntent {
            kind: TM_ADMISSION_SHELL_INTENT_EMIT_TRANSACTION_ADDED,
            hash: outcome.inserted_hash,
        });
    }

    TransactionManagerAdmissionCommandReport {
        inserted_hash_found: outcome.inserted_hash_found,
        inserted_hash: outcome.inserted_hash,
        transaction_added_hash_found: outcome.emit_transaction_added && outcome.inserted_hash_found,
        transaction_added_hash: outcome.inserted_hash,
        shell_intents,
        admission: command_admission_result(outcome),
    }
}

fn public_insert_verify_result(
    status: u8,
    chain_id: u64,
    expected_chain_id: u64,
) -> TransactionManagerPublicInsertResult {
    let message = match status {
        TM_VERIFY_TRANSACTION_STATUS_ACCEPTED => "",
        TM_VERIFY_TRANSACTION_STATUS_CHAIN_ID_MISMATCH => {
            return TransactionManagerPublicInsertResult {
                accepted: false,
                message: format!("chain_id mismatch {chain_id} {expected_chain_id}"),
            };
        }
        TM_VERIFY_TRANSACTION_STATUS_INVALID_GAS => "invalid gas",
        TM_VERIFY_TRANSACTION_STATUS_INTRINSIC_GAS => "intrinsic gas too low",
        TM_VERIFY_TRANSACTION_STATUS_INVALID_SIGNATURE => "invalid signature",
        TM_VERIFY_TRANSACTION_STATUS_GAS_PRICE => "gas_price too low",
        _ => "unknown transaction verification status",
    };
    TransactionManagerPublicInsertResult {
        accepted: status == TM_VERIFY_TRANSACTION_STATUS_ACCEPTED,
        message: message.to_string(),
    }
}

fn public_insert_admission_result(
    admission: &TransactionManagerAdmissionResult,
) -> TransactionManagerPublicInsertResult {
    match admission.insert_status {
        TM_INSERT_TRANSACTION_STATUS_ACCEPTED => TransactionManagerPublicInsertResult {
            accepted: true,
            message: "".to_string(),
        },
        TM_INSERT_TRANSACTION_STATUS_ALREADY_KNOWN => TransactionManagerPublicInsertResult {
            accepted: false,
            message: "Transaction already in transactions pool".to_string(),
        },
        TM_INSERT_TRANSACTION_STATUS_ALREADY_FINALIZED => TransactionManagerPublicInsertResult {
            accepted: false,
            message: format!(
                "Transaction already finalized in period{}",
                admission.finalized_period
            ),
        },
        TM_INSERT_TRANSACTION_STATUS_CANNOT_INSERT => TransactionManagerPublicInsertResult {
            accepted: false,
            message: "Transaction could not be inserted".to_string(),
        },
        _ => TransactionManagerPublicInsertResult {
            accepted: false,
            message: "Transaction could not be inserted".to_string(),
        },
    }
}

fn public_admission_command_report(
    verification_status: u8,
    verify_fact: &TransactionManagerVerifyTransactionFact,
    admission: TransactionManagerAdmissionCommandReport,
    public_result: TransactionManagerPublicInsertResult,
) -> TransactionManagerPublicAdmissionCommandReport {
    TransactionManagerPublicAdmissionCommandReport {
        verification_status,
        verification_chain_id: verify_fact.chain_id,
        verification_expected_chain_id: verify_fact.expected_chain_id,
        public_result,
        admission,
    }
}

fn public_precheck_rejected_command_report(
    precheck: TransactionManagerInsertTransactionOutcome,
    verify_fact: &TransactionManagerVerifyTransactionFact,
) -> TransactionManagerPublicAdmissionCommandReport {
    let admission = TransactionManagerAdmissionCommandReport {
        inserted_hash_found: false,
        inserted_hash: [0; 32],
        transaction_added_hash_found: false,
        transaction_added_hash: [0; 32],
        shell_intents: Vec::new(),
        admission: command_admission_result_from_insert_outcome(&precheck),
    };
    let public_result = public_insert_admission_result(&admission.admission);
    public_admission_command_report(
        TM_VERIFY_TRANSACTION_STATUS_ACCEPTED,
        verify_fact,
        admission,
        public_result,
    )
}

fn public_verification_rejected_command_report(
    verify_status: u8,
    verify_fact: &TransactionManagerVerifyTransactionFact,
) -> TransactionManagerPublicAdmissionCommandReport {
    let admission = TransactionManagerAdmissionCommandReport {
        inserted_hash_found: false,
        inserted_hash: [0; 32],
        transaction_added_hash_found: false,
        transaction_added_hash: [0; 32],
        shell_intents: Vec::new(),
        admission: TransactionManagerAdmissionResult {
            present: false,
            insert_status: TM_INSERT_TRANSACTION_STATUS_ACCEPTED,
            transaction_status: 0,
            finalized_period_known: false,
            finalized_period: 0,
            requires_finalized_lookup: false,
        },
    };
    public_admission_command_report(
        verify_status,
        verify_fact,
        admission,
        public_insert_verify_result(
            verify_status,
            verify_fact.chain_id,
            verify_fact.expected_chain_id,
        ),
    )
}

fn finalized_status_command_report(
    outcome: &FinalizedTransactionStatusPlan,
) -> TransactionManagerFinalizedStatusCommandReport {
    let mut removed_non_finalized = Vec::new();
    let mut queue_erased = Vec::new();
    for action in &outcome.accepted {
        if action.removed_non_finalized {
            removed_non_finalized.push(hash_command(action.hash));
        }
        if action.erase_from_queue && action.erased_from_queue {
            queue_erased.push(hash_command(action.hash));
        }
    }
    TransactionManagerFinalizedStatusCommandReport {
        removed_non_finalized,
        queue_erased,
        finalized_account_purged: Vec::new(),
        accepted_count: outcome.accepted.len() as u64,
        purge_transaction_queue: outcome.purge_transaction_queue,
    }
}

fn append_queue_cleanup_to_finalized_status_command_report(
    report: &mut TransactionManagerFinalizedStatusCommandReport,
    cleanup: TransactionManagerRuntimeQueueCleanupPlan,
) {
    report.queue_erased.extend(
        cleanup
            .non_proposable_expired
            .removed_hashes
            .into_iter()
            .map(|entry| hash_command(entry.hash)),
    );
    report.finalized_account_purged.extend(
        cleanup
            .finalized_account_purged
            .removed_hashes
            .into_iter()
            .map(|entry| hash_command(entry.hash)),
    );
}

fn runtime_queue_entry_from_insert_input(
    input: &TransactionQueueInsertInput,
) -> TransactionQueueEntry {
    TransactionQueueEntry {
        hash: H256::from(input.hash),
        sender: H160::from(input.sender),
        nonce: U256::from_big_endian(&input.nonce),
        gas_price: U256::from_big_endian(&input.gas_price),
        gas: input.gas,
        data_size: input.data_size as u64,
        rlp: input.tx_rlp.clone(),
        last_block_number: input.last_block_number,
    }
}

fn runtime_queue_stored_transaction_from_entry(
    entry: Option<TransactionQueueEntry>,
) -> TransactionQueueStoredTransaction {
    if let Some(entry) = entry {
        TransactionQueueStoredTransaction {
            found: true,
            hash: entry.hash.0,
            tx_rlp: entry.rlp,
        }
    } else {
        TransactionQueueStoredTransaction {
            found: false,
            hash: [0; 32],
            tx_rlp: Vec::new(),
        }
    }
}

pub(crate) fn service_transaction_groups_to_bridge(
    groups: Vec<Vec<TransactionQueueEntry>>,
) -> Vec<TransactionQueueTransactionGroup> {
    groups
        .into_iter()
        .map(|transactions| TransactionQueueTransactionGroup {
            transactions: transactions
                .into_iter()
                .map(Some)
                .map(runtime_queue_stored_transaction_from_entry)
                .collect(),
        })
        .collect()
}

fn runtime_hashes_to_bridge(hashes: Vec<H256>) -> Vec<TransactionQueueHash> {
    hashes
        .into_iter()
        .map(|hash| TransactionQueueHash { hash: hash.0 })
        .collect()
}

fn runtime_queue_purge_plan_from_consensus(
    outcome: TransactionQueuePurgeOutcome,
) -> TransactionManagerRuntimeQueuePurgePlan {
    TransactionManagerRuntimeQueuePurgePlan {
        removed_hashes: runtime_hashes_to_bridge(outcome.removed_hashes),
    }
}

fn runtime_queue_account_nonce_facts_from_bridge(
    proposable_accounts: Vec<H160>,
    account_nonce_facts: Vec<BridgeTransactionQueueAccountNonceFact>,
) -> Vec<TransactionQueueAccountNonceFact> {
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

    proposable_accounts
        .into_iter()
        .map(|sender| {
            let (account_found, account_nonce) = account_nonce_facts
                .get(&sender)
                .copied()
                .unwrap_or((false, U256::zero()));
            TransactionQueueAccountNonceFact {
                sender,
                account_found,
                account_nonce,
            }
        })
        .collect()
}

fn transaction_manager_runtime_storage(runtime: &TransactionServiceState) -> Result<&Storage> {
    Ok(runtime.storage.as_ref())
}

/// Plans and persists accepted DAG-block transactions through the Rust manager runtime.
fn save_transactions_from_dag_block_with_runtime(
    runtime: &mut TransactionServiceState,
    facts: Vec<DagTransactionSaveSidecarFact>,
) -> Result<DagTransactionSaveOutcome> {
    let storage = transaction_manager_runtime_storage(runtime)?;
    let prepared = prepare_transactions_from_dag_block_with_runtime(runtime, facts)?;
    let publication = prepare_dag_transaction_publication(runtime, &prepared)?;
    if !prepared.accepted_payloads.is_empty() {
        let mut batch = storage.create_write_batch();
        append_prepared_dag_transactions_to_batch(storage, &mut batch, &prepared)?;
        storage
            .commit_write_batch_with_sync(batch, false)
            .context("TM_DAG_TX_BATCH_COMMIT")?;
    }
    Ok(publish_prepared_dag_transactions(runtime, publication))
}

/// Plans DAG transaction persistence without mutating storage or live runtime state.
pub(crate) fn prepare_transactions_from_dag_block_with_runtime(
    runtime: &TransactionServiceState,
    facts: Vec<DagTransactionSaveSidecarFact>,
) -> Result<PreparedDagTransactionSave> {
    let storage = transaction_manager_runtime_storage(runtime)?;
    let plan = plan_transactions_from_dag_block(
        facts
            .into_iter()
            .map(|fact| {
                let hash = H256::from(fact.hash);
                ConsensusDagTransactionSaveFact {
                    input_index: fact.input_index,
                    hash,
                    trx_rlp: fact.trx_rlp,
                    transaction_nonce: U256::from_big_endian(&fact.transaction_nonce),
                    sender_account_nonce: U256::from_big_endian(&fact.sender_account_nonce),
                    in_non_finalized_cache: runtime.sidecar.contains_non_finalized(hash),
                    in_recently_finalized_cache: runtime.sidecar.contains_recently_finalized(hash),
                }
            })
            .collect(),
        runtime.sidecar.transaction_count(),
        |hash| transaction_finalized(storage, hash).context("TM_DAG_TX_FINALIZED_LOOKUP_FAILED"),
    )?;

    let mut accepted = Vec::with_capacity(plan.accepted_transactions.len());
    let mut accepted_payloads = Vec::with_capacity(plan.accepted_transactions.len());

    for payload in &plan.accepted_transactions {
        accepted.push(DagTransactionSaveAccepted {
            hash: payload.hash.0,
            erased_from_queue: false,
        });
        accepted_payloads.push(NonFinalizedTransactionStoragePayload {
            hash: payload.hash,
            trx_rlp: payload.trx_rlp.clone(),
        });
    }

    Ok(PreparedDagTransactionSave {
        accepted,
        accepted_payloads,
        target_transaction_count: plan.target_transaction_count,
    })
}

/// Appends a prepared DAG transaction save to a caller-owned shared batch.
pub(crate) fn append_prepared_dag_transactions_to_batch(
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

/// Preapplies a DAG transaction save to cloned queue and sidecar state.
pub(crate) fn prepare_dag_transaction_publication(
    runtime: &TransactionServiceState,
    prepared: &PreparedDagTransactionSave,
) -> Result<PreparedDagTransactionPublication> {
    let mut queue = runtime.queue.clone();
    let mut sidecar = runtime.sidecar.clone();
    let mut accepted = prepared
        .accepted
        .iter()
        .map(|entry| DagTransactionSaveAccepted {
            hash: entry.hash,
            erased_from_queue: false,
        })
        .collect::<Vec<_>>();
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

/// Publishes a fully prevalidated DAG transaction live-state transition.
pub(crate) fn publish_prepared_dag_transactions(
    runtime: &mut TransactionServiceState,
    publication: PreparedDagTransactionPublication,
) -> DagTransactionSaveOutcome {
    runtime.queue = publication.queue;
    runtime.sidecar = publication.sidecar;
    publication.outcome
}

/// Applies DAG transaction persistence and returns a typed command report.
pub fn save_transactions_from_dag_block_command_report_with_runtime(
    runtime: &mut TransactionServiceState,
    facts: Vec<DagTransactionSaveSidecarFact>,
) -> Result<TransactionManagerDagSaveCommandReport> {
    let outcome = save_transactions_from_dag_block_with_runtime(runtime, facts)?;
    Ok(dag_save_command_report(&outcome))
}

/// Plans and applies finalized transaction status updates through the Rust manager runtime.
///
/// Rust persists count changes before mutating live runtime state. Once storage
/// succeeds, the runtime evicts stale recent-finalized sidecars, inserts current
/// finalized payloads, marks queue-known membership, erases matching queued
/// payloads, and advances the authoritative transaction count.
fn update_finalized_transactions_status_with_runtime(
    runtime: &mut TransactionServiceState,
    period: u64,
    retention_window: u64,
    facts: Vec<FinalizedTransactionStatusSidecarFact>,
) -> Result<FinalizedTransactionStatusPlan> {
    let consensus_facts = facts
        .iter()
        .map(|fact| {
            let hash = H256::from(fact.hash);
            ConsensusFinalizedTransactionStatusFact {
                input_index: fact.input_index,
                hash,
                in_non_finalized_cache: runtime.sidecar.contains_non_finalized(hash),
            }
        })
        .collect();

    let plan: ConsensusFinalizedTransactionStatusPlan = plan_finalized_transactions_status(
        consensus_facts,
        runtime.sidecar.transaction_count(),
        period,
        retention_window,
    )?;

    if !plan.accepted_transactions.is_empty() {
        save_transaction_count(
            transaction_manager_runtime_storage(runtime)?,
            plan.target_transaction_count,
        )
        .context("TM_FINALIZED_STATUS_TRXCOUNT_WRITE")?;
    }

    if let Some(stale_period) = plan.stale_period {
        runtime
            .sidecar
            .evict_recently_finalized_stale_period(stale_period);
    }

    let mut accepted = Vec::with_capacity(plan.accepted_transactions.len());
    for action in &plan.accepted_transactions {
        let fact = facts
            .get(action.input_index as usize)
            .context("TM_RUNTIME_FINALIZED_STATUS_INPUT_INDEX")?;
        let hash = H256::from(fact.hash);
        ensure!(
            hash == action.hash,
            "TM_RUNTIME_FINALIZED_STATUS_HASH_MISMATCH"
        );
        runtime
            .sidecar
            .insert_recently_finalized(period, hash, fact.trx_rlp.clone())
            .context("TM_RUNTIME_FINALIZED_STATUS_INSERT")?;
        runtime.queue.mark_transaction_known(hash);
        let erased_from_queue = runtime.queue.erase(hash);
        accepted.push(FinalizedTransactionStatusAction {
            hash: action.hash.0,
            removed_non_finalized: action.removed_non_finalized,
            erase_from_queue: true,
            erased_from_queue,
        });
    }
    runtime
        .sidecar
        .set_transaction_count(plan.target_transaction_count);

    Ok(FinalizedTransactionStatusPlan {
        accepted,
        purge_transaction_queue: plan.purge_transactions,
    })
}

/// Applies finalized-transaction status changes and returns typed command actions.
#[cfg(test)]
pub fn update_finalized_transactions_status_command_report_with_runtime(
    runtime: &mut TransactionServiceState,
    period: u64,
    retention_window: u64,
    facts: Vec<FinalizedTransactionStatusSidecarFact>,
) -> Result<TransactionManagerFinalizedStatusCommandReport> {
    let outcome = update_finalized_transactions_status_with_runtime(
        runtime,
        period,
        retention_window,
        facts,
    )?;
    Ok(finalized_status_command_report(&outcome))
}

/// Applies finalized status changes plus queue purge and returns typed command actions.
pub fn update_finalized_transactions_status_command_report_with_runtime_and_account_nonce_facts(
    runtime: &mut TransactionServiceState,
    period: u64,
    retention_window: u64,
    account_nonce_facts: Vec<BridgeTransactionQueueAccountNonceFact>,
    facts: Vec<FinalizedTransactionStatusSidecarFact>,
) -> Result<TransactionManagerFinalizedStatusCommandReport> {
    let outcome = update_finalized_transactions_status_with_runtime(
        runtime,
        period,
        retention_window,
        facts,
    )?;
    let mut report = finalized_status_command_report(&outcome);
    if report.purge_transaction_queue {
        let mut runtime = TransactionRuntimeAccess(runtime);
        let cleanup = runtime.transaction_manager_runtime_queue_cleanup_with_account_nonce_facts(
            false,
            0,
            account_nonce_facts,
        )?;
        append_queue_cleanup_to_finalized_status_command_report(&mut report, cleanup);
        report.purge_transaction_queue = false;
    }
    Ok(report)
}

/// Builds a deterministic admission plan for C++ pre-admission verification.
pub fn transaction_manager_verify_transaction(
    fact: TransactionManagerVerifyTransactionFact,
) -> Result<TransactionManagerVerifyTransactionOutcome> {
    let outcome = plan_verify_transaction(consensus_verify_transaction_fact_from_ffi_fact(fact))?;
    Ok(TransactionManagerVerifyTransactionOutcome {
        status: match outcome.status {
            TransactionManagerVerifyTransactionStatus::Accepted => {
                TM_VERIFY_TRANSACTION_STATUS_ACCEPTED
            }
            TransactionManagerVerifyTransactionStatus::ChainIdMismatch => {
                TM_VERIFY_TRANSACTION_STATUS_CHAIN_ID_MISMATCH
            }
            TransactionManagerVerifyTransactionStatus::InvalidGas => {
                TM_VERIFY_TRANSACTION_STATUS_INVALID_GAS
            }
            TransactionManagerVerifyTransactionStatus::IntrinsicGasNotCovered => {
                TM_VERIFY_TRANSACTION_STATUS_INTRINSIC_GAS
            }
            TransactionManagerVerifyTransactionStatus::InvalidSignature => {
                TM_VERIFY_TRANSACTION_STATUS_INVALID_SIGNATURE
            }
            TransactionManagerVerifyTransactionStatus::GasPriceTooLow => {
                TM_VERIFY_TRANSACTION_STATUS_GAS_PRICE
            }
        },
    })
}

/// Builds the insertion status mapping used by Rust-owned admission command reports.
fn transaction_manager_insert_transaction(
    fact: TransactionManagerInsertTransactionFact,
) -> Result<TransactionManagerInsertTransactionOutcome> {
    let outcome = plan_insert_transaction(
        consensus_insert_transaction_fact_from_ffi_fact(fact)
            .context("TM_TX_INSERT_FACT_CONVERSION_FAILED")?,
    )?;

    Ok(match outcome.status {
        TransactionManagerInsertTransactionStatus::Accepted => {
            TransactionManagerInsertTransactionOutcome {
                status: TM_INSERT_TRANSACTION_STATUS_ACCEPTED,
                finalized_period: 0,
                finalized_period_known: false,
            }
        }
        TransactionManagerInsertTransactionStatus::AlreadyKnown => {
            TransactionManagerInsertTransactionOutcome {
                status: TM_INSERT_TRANSACTION_STATUS_ALREADY_KNOWN,
                finalized_period: 0,
                finalized_period_known: false,
            }
        }
        TransactionManagerInsertTransactionStatus::AlreadyFinalized => {
            TransactionManagerInsertTransactionOutcome {
                status: TM_INSERT_TRANSACTION_STATUS_ALREADY_FINALIZED,
                finalized_period: outcome.finalized_period.unwrap_or_default(),
                finalized_period_known: true,
            }
        }
        TransactionManagerInsertTransactionStatus::CouldNotInsert => {
            TransactionManagerInsertTransactionOutcome {
                status: TM_INSERT_TRANSACTION_STATUS_CANNOT_INSERT,
                finalized_period: 0,
                finalized_period_known: false,
            }
        }
    })
}

/// Creates the Rust-owned TransactionManager runtime for Rust-enabled manager shims.
///
/// The runtime owns both the live manager sidecars and the transaction queue
/// metadata/payload state. C++ supplies materialized transaction facts at method
/// boundaries and remains responsible for events, logging, historical account
/// reads, and gas estimation. Latest-state admission, DAG-save, verification,
/// and finalized-account queue purge can source account facts directly from
/// Rust FinalChain through runtime APIs.
#[cfg(test)]
fn build_transaction_state_for_test(
    initial_transaction_count: u64,
    config: TransactionQueueConfig,
) -> Box<TransactionRuntimeState> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be available")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "rustaxa_bridge_transaction_runtime_{initial_transaction_count}_{nonce}"
    ));
    let storage = std::sync::Arc::new(
        Storage::new(rustaxa_storage::Config::new(path.clone()))
            .expect("test transaction storage should open"),
    );
    storage
        .metadata()
        .write_status_field(StatusField::TrxCount as u8, initial_transaction_count)
        .expect("test transaction count should persist");
    let state = TransactionServiceState::restore(
        storage,
        TransactionServiceConfig {
            queue_max_size: config.max_size,
            gas_pricer_config: test_gas_pricer_config(),
            proposal_dag_gas_limit: u64::MAX,
        },
    )
    .expect("test transaction state should restore");
    Box::new(TransactionRuntimeAccess(TestTransactionServiceState {
        state: Some(Box::new(state)),
        cleanup_path: Some(path),
    }))
}

/// Converts the flat CXX gas-pricer configuration into the native policy type.
///
/// The conversion is infallible and preserves all scalar values; validation is
/// performed by the native transaction application root before publication.
pub(crate) fn domain_gas_pricer_config(config: GasPricerConfig) -> DomainGasPricerConfig {
    DomainGasPricerConfig {
        percentile: config.percentile,
        minimum_price: U256::from_big_endian(&config.minimum_price),
        history_blocks: config.history_blocks,
        is_light_node: config.is_light_node,
        blocks_gas_pricer: config.blocks_gas_pricer,
    }
}

#[cfg(test)]
fn test_gas_pricer_config() -> DomainGasPricerConfig {
    DomainGasPricerConfig {
        percentile: 50,
        minimum_price: U256::one(),
        history_blocks: 10,
        is_light_node: false,
        blocks_gas_pricer: false,
    }
}

impl<T> TransactionRuntimeAccess<T>
where
    T: DerefMut<Target = TransactionServiceState>,
{
    /// Inserts or updates one live non-finalized sidecar payload.
    #[cfg(test)]
    fn transaction_manager_runtime_insert_non_finalized(
        &mut self,
        input: TransactionManagerSidecarInsertInput,
    ) -> Result<()> {
        self.sidecar
            .insert_non_finalized(H256::from(input.hash), input.trx_rlp)
            .context("TM_RUNTIME_INSERT_NON_FINALIZED")
    }

    /// True when hash exists in non-finalized sidecar state.
    #[cfg(test)]
    fn transaction_manager_runtime_contains_non_finalized(&self, hash: &[u8; 32]) -> bool {
        self.sidecar.contains_non_finalized(H256::from(*hash))
    }

    /// True when hash exists in recently-finalized sidecar state.
    #[cfg(test)]
    fn transaction_manager_runtime_contains_recently_finalized(&self, hash: &[u8; 32]) -> bool {
        self.sidecar.contains_recently_finalized(H256::from(*hash))
    }

    /// Inserts transaction metadata and canonical bytes into the Rust-owned queue.
    pub(crate) fn transaction_manager_runtime_queue_insert(
        &mut self,
        input: TransactionQueueInsertInput,
    ) -> Result<TransactionManagerRuntimeQueueInsertOutcome> {
        let proposable = input.proposable;
        let outcome = self
            .0
            .queue
            .insert(runtime_queue_entry_from_insert_input(&input), proposable)?;
        if matches!(outcome.status, TransactionQueueInsertStatus::Overflow)
            || !outcome.overflow_removed_hashes.is_empty()
        {
            self.last_drop_observed = Some(Instant::now());
        }
        Ok(TransactionManagerRuntimeQueueInsertOutcome {
            status: queue_status_to_ffi(outcome.status),
            inserted_hash_found: outcome.inserted_hash.is_some(),
            inserted_hash: outcome.inserted_hash.unwrap_or_default().0,
            demoted_hashes: runtime_hashes_to_bridge(outcome.demoted_hashes),
            overflow_removed_hashes: runtime_hashes_to_bridge(outcome.overflow_removed_hashes),
        })
    }

    /// Returns the Rust-owned public insert precheck for known transactions.
    ///
    /// Public admission commands call this before signature/gas verification so
    /// known hashes keep the legacy fast path while Rust remains authoritative
    /// for queue-known plus sidecar membership.
    fn transaction_manager_runtime_insert_transaction_precheck(
        &self,
        hash: &[u8; 32],
    ) -> Result<TransactionManagerInsertTransactionOutcome> {
        let tx_hash = H256::from(*hash);
        let hash_known = self
            .0
            .sidecar
            .is_transaction_known(TransactionManagerKnownFact {
                hash: tx_hash,
                queue_known: self.queue.is_transaction_known(tx_hash),
            })
            .context("TM_RUNTIME_INSERT_PRECHECK_KNOWN_CHECK_FAILED")?;
        transaction_manager_insert_transaction(TransactionManagerInsertTransactionFact {
            tx_hash: *hash,
            hash_known,
            queue_status: TransactionQueueInsertStatus::Inserted as u8,
            has_finalized_period: false,
            finalized_period: 0,
        })
    }

    /// Executes TransactionManager admission using account/finalization facts
    /// supplied by the C++ external-EVM boundary.
    ///
    /// The deterministic admission decision, queue mutation, and public status
    /// mapping remain Rust-owned. C++ supplies only the account and finalized
    /// transaction facts that are still owned by the external EVM/FinalChain
    /// compatibility shell.
    fn transaction_manager_runtime_execute_transaction_admission_with_final_chain_facts(
        &mut self,
        fact: TransactionManagerValidatedInsertRuntimeFact,
        final_chain_fact: TransactionManagerFinalChainAdmissionFact,
        mut input: TransactionQueueInsertInput,
    ) -> Result<TransactionManagerRuntimeAdmissionOutcome> {
        ensure!(
            input.hash == fact.tx_hash,
            "TM_RUNTIME_VALIDATED_INSERT_HASH_MISMATCH"
        );
        ensure!(
            input.nonce == fact.transaction_nonce,
            "TM_RUNTIME_VALIDATED_INSERT_NONCE_MISMATCH"
        );
        ensure!(
            input.gas == fact.gas_limit,
            "TM_RUNTIME_VALIDATED_INSERT_GAS_MISMATCH"
        );
        let hash = H256::from(fact.tx_hash);
        let plan = plan_validated_insert(ConsensusTransactionManagerValidatedInsertFact {
            tx_hash: hash,
            transaction_nonce: U256::from_big_endian(&fact.transaction_nonce),
            transaction_cost: U256::from_big_endian(&fact.transaction_cost),
            gas_limit: fact.gas_limit,
            propose_dag_gas_limit: fact.propose_dag_gas_limit,
            insert_non_proposable: fact.insert_non_proposable,
            in_non_finalized_cache: self.sidecar.contains_non_finalized(hash),
            in_recently_finalized_cache: self.sidecar.contains_recently_finalized(hash),
            account_found: final_chain_fact.account_found,
            account_nonce: U256::from_big_endian(&final_chain_fact.account_nonce),
            account_balance: U256::from_big_endian(&final_chain_fact.account_balance),
        })?;

        let queue_outcome = if plan.should_insert_queue {
            input.proposable = plan.queue_proposable;
            self.transaction_manager_runtime_queue_insert(input)?
        } else {
            TransactionManagerRuntimeQueueInsertOutcome {
                status: queue_status_to_ffi(plan.status),
                inserted_hash_found: false,
                inserted_hash: [0; 32],
                demoted_hashes: Vec::new(),
                overflow_removed_hashes: Vec::new(),
            }
        };
        let insert_outcome =
            transaction_manager_insert_transaction(TransactionManagerInsertTransactionFact {
                tx_hash: fact.tx_hash,
                hash_known: false,
                queue_status: queue_outcome.status,
                has_finalized_period: final_chain_fact.finalized_period_known,
                finalized_period: final_chain_fact.finalized_period,
            })?;

        Ok(TransactionManagerRuntimeAdmissionOutcome {
            insert_status: insert_outcome.status,
            transaction_status: queue_outcome.status,
            requires_finalized_lookup: false,
            finalized_period_known: insert_outcome.finalized_period_known,
            finalized_period: insert_outcome.finalized_period,
            emit_transaction_added: plan.emit_transaction_added
                && queue_outcome.status == TransactionQueueInsertStatus::Inserted as u8,
            inserted_hash_found: queue_outcome.inserted_hash_found,
            inserted_hash: queue_outcome.inserted_hash,
        })
    }

    /// Executes fact-backed admission and returns a typed command report.
    pub fn transaction_manager_runtime_execute_transaction_admission_with_final_chain_facts_command_report(
        &mut self,
        fact: TransactionManagerValidatedInsertRuntimeFact,
        final_chain_fact: TransactionManagerFinalChainAdmissionFact,
        input: TransactionQueueInsertInput,
    ) -> Result<TransactionManagerAdmissionCommandReport> {
        let outcome = self
            .transaction_manager_runtime_execute_transaction_admission_with_final_chain_facts(
                fact,
                final_chain_fact,
                input,
            )?;
        Ok(admission_command_report(&outcome))
    }

    /// Executes public insert precheck, verification, and fact-backed admission.
    pub fn transaction_manager_runtime_execute_public_transaction_admission_with_final_chain_facts_command_report(
        &mut self,
        verify_fact: TransactionManagerVerifyTransactionFact,
        admission_fact: TransactionManagerValidatedInsertRuntimeFact,
        final_chain_fact: TransactionManagerFinalChainAdmissionFact,
        input: TransactionQueueInsertInput,
    ) -> Result<TransactionManagerPublicAdmissionCommandReport> {
        let tx_hash = verify_fact.tx_hash;
        let verification_chain_id = verify_fact.chain_id;
        let verification_expected_chain_id = verify_fact.expected_chain_id;
        ensure!(
            tx_hash == admission_fact.tx_hash,
            "TM_RUNTIME_PUBLIC_INSERT_VERIFY_HASH_MISMATCH"
        );
        let precheck = self.transaction_manager_runtime_insert_transaction_precheck(&tx_hash)?;
        if precheck.status != TM_INSERT_TRANSACTION_STATUS_ACCEPTED {
            return Ok(public_precheck_rejected_command_report(
                precheck,
                &verify_fact,
            ));
        }

        let verify_outcome = transaction_manager_verify_transaction(verify_fact)?;
        if verify_outcome.status != TM_VERIFY_TRANSACTION_STATUS_ACCEPTED {
            let verify_fact = TransactionManagerVerifyTransactionFact {
                tx_hash,
                chain_id: verification_chain_id,
                expected_chain_id: verification_expected_chain_id,
                gas_limit: 0,
                max_gas_limit: 0,
                last_block_number: 0,
                cornus_active: false,
                intrinsic_gas_covered: true,
                signature_valid: true,
                gas_price: [0; 32],
                minimum_gas_price: [0; 32],
            };
            return Ok(public_verification_rejected_command_report(
                verify_outcome.status,
                &verify_fact,
            ));
        }

        let admission = self
            .transaction_manager_runtime_execute_transaction_admission_with_final_chain_facts_command_report(
                admission_fact,
                final_chain_fact,
                input,
            )?;
        let public_result = public_insert_admission_result(&admission.admission);
        Ok(public_admission_command_report(
            TM_VERIFY_TRANSACTION_STATUS_ACCEPTED,
            &TransactionManagerVerifyTransactionFact {
                tx_hash,
                chain_id: verification_chain_id,
                expected_chain_id: verification_expected_chain_id,
                gas_limit: 0,
                max_gas_limit: 0,
                last_block_number: 0,
                cornus_active: false,
                intrinsic_gas_covered: true,
                signature_valid: true,
                gas_price: [0; 32],
                minimum_gas_price: [0; 32],
            },
            admission,
            public_result,
        ))
    }

    /// Returns true when the queue contains a transaction hash.
    #[cfg(test)]
    fn transaction_manager_runtime_queue_contains(&self, hash: &[u8; 32]) -> bool {
        self.queue.contains(H256::from(*hash))
    }

    /// Applies Rust-owned queue cleanup using proposal-account nonce facts.
    fn transaction_manager_runtime_queue_cleanup_with_account_nonce_facts(
        &mut self,
        apply_block_finalized: bool,
        block_number: u64,
        account_nonce_facts: Vec<BridgeTransactionQueueAccountNonceFact>,
    ) -> Result<TransactionManagerRuntimeQueueCleanupPlan> {
        let account_nonce_facts = runtime_queue_account_nonce_facts_from_bridge(
            self.queue.proposable_accounts(),
            account_nonce_facts,
        );
        let non_proposable_expired = if apply_block_finalized {
            self.queue.block_finalized_plan(block_number)
        } else {
            TransactionQueuePurgeOutcome::default()
        };
        let finalized_account_purged = self.queue.purge_accounts_plan(&account_nonce_facts);
        Ok(TransactionManagerRuntimeQueueCleanupPlan {
            non_proposable_expired: runtime_queue_purge_plan_from_consensus(non_proposable_expired),
            finalized_account_purged: runtime_queue_purge_plan_from_consensus(
                finalized_account_purged,
            ),
        })
    }
}

#[cfg(test)]
impl TransactionRuntimeAccess<TestTransactionServiceState> {
    fn transaction_manager_runtime_transaction_count(&self) -> u64 {
        self.sidecar.transaction_count()
    }

    fn transaction_manager_runtime_queue_size(&self) -> usize {
        self.queue.size() as usize
    }

    fn transaction_manager_runtime_queue_transactions_dropped(&self) -> bool {
        self.last_drop_observed
            .is_some_and(|observed| observed.elapsed() < TRANSACTION_QUEUE_DROP_WINDOW)
    }
}

fn consensus_verify_transaction_fact_from_ffi_fact(
    fact: TransactionManagerVerifyTransactionFact,
) -> ConsensusTransactionManagerVerifyTransactionFact {
    ConsensusTransactionManagerVerifyTransactionFact {
        tx_hash: H256::from(fact.tx_hash),
        chain_id: fact.chain_id,
        expected_chain_id: fact.expected_chain_id,
        gas_limit: fact.gas_limit,
        max_gas_limit: fact.max_gas_limit,
        last_block_number: fact.last_block_number,
        cornus_active: fact.cornus_active,
        intrinsic_gas_covered: fact.intrinsic_gas_covered,
        signature_valid: fact.signature_valid,
        gas_price: U256::from_big_endian(&fact.gas_price),
        minimum_gas_price: U256::from_big_endian(&fact.minimum_gas_price),
    }
}

fn consensus_insert_transaction_fact_from_ffi_fact(
    fact: TransactionManagerInsertTransactionFact,
) -> Result<ConsensusTransactionManagerInsertTransactionFact> {
    Ok(ConsensusTransactionManagerInsertTransactionFact {
        tx_hash: H256::from(fact.tx_hash),
        hash_known: fact.hash_known,
        queue_status: queue_status_from_ffi(fact.queue_status)?,
        has_finalized_period: fact.has_finalized_period,
        finalized_period: fact.finalized_period,
    })
}

fn queue_status_to_ffi(status: TransactionQueueInsertStatus) -> u8 {
    match status {
        TransactionQueueInsertStatus::Inserted => TransactionQueueInsertStatus::Inserted as u8,
        TransactionQueueInsertStatus::InsertedNonProposable => {
            TransactionQueueInsertStatus::InsertedNonProposable as u8
        }
        TransactionQueueInsertStatus::Known => TransactionQueueInsertStatus::Known as u8,
        TransactionQueueInsertStatus::Overflow => TransactionQueueInsertStatus::Overflow as u8,
    }
}

fn queue_status_from_ffi(status: u8) -> Result<TransactionQueueInsertStatus> {
    Ok(match status {
        x if x == TransactionQueueInsertStatus::Inserted as u8 => {
            TransactionQueueInsertStatus::Inserted
        }
        x if x == TransactionQueueInsertStatus::InsertedNonProposable as u8 => {
            TransactionQueueInsertStatus::InsertedNonProposable
        }
        x if x == TransactionQueueInsertStatus::Known as u8 => TransactionQueueInsertStatus::Known,
        x if x == TransactionQueueInsertStatus::Overflow as u8 => {
            TransactionQueueInsertStatus::Overflow
        }
        _ => {
            anyhow::bail!("unknown transaction queue status: {}", status)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::{BridgeMetadataStorageQueries, BridgeStorage};
    use crate::storage::create_metadata_storage_queries;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn bridge_gas_pricer_config(blocks_gas_pricer: bool) -> GasPricerConfig {
        GasPricerConfig {
            percentile: 50,
            minimum_price: U256::one().to_big_endian(),
            history_blocks: 10,
            is_light_node: false,
            blocks_gas_pricer,
        }
    }

    fn build_transaction_state_from_storage(
        storage: &BridgeStorage,
        config: TransactionQueueConfig,
    ) -> Result<Box<TransactionRuntimeState>> {
        let state = TransactionServiceState::restore(
            storage.0.clone(),
            TransactionServiceConfig {
                queue_max_size: config.max_size,
                gas_pricer_config: domain_gas_pricer_config(bridge_gas_pricer_config(false)),
                proposal_dag_gas_limit: 1_000_000,
            },
        )?;
        Ok(Box::new(TransactionRuntimeAccess(
            TestTransactionServiceState {
                state: Some(Box::new(state)),
                cleanup_path: None,
            },
        )))
    }

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn metadata_queries(storage: &BridgeStorage) -> Box<BridgeMetadataStorageQueries> {
        create_metadata_storage_queries(storage)
    }

    #[test]
    fn bridge_transaction_manager_runtime_from_storage_defaults_missing_count_to_zero() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_default_count");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        let runtime =
            build_transaction_state_from_storage(&storage, TransactionQueueConfig { max_size: 16 })
                .expect("runtime should restore the storage default");

        assert_eq!(runtime.transaction_manager_runtime_transaction_count(), 0);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_runtime_from_storage_restores_persisted_count() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_persisted_count");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        storage
            .0
            .metadata()
            .write_status_field(StatusField::TrxCount as u8, 73)
            .expect("transaction count should persist");

        let runtime =
            build_transaction_state_from_storage(&storage, TransactionQueueConfig { max_size: 16 })
                .expect("runtime should restore the persisted count");

        assert_eq!(runtime.transaction_manager_runtime_transaction_count(), 73);
        let _ = fs::remove_dir_all(temp_dir);
    }

    fn dag_tx_sidecar_fact(
        input_index: u64,
        hash: u8,
        tx_nonce: u64,
        sender_nonce: u64,
        rlp: u8,
    ) -> DagTransactionSaveSidecarFact {
        DagTransactionSaveSidecarFact {
            input_index,
            hash: [hash; 32],
            trx_rlp: vec![rlp],
            transaction_nonce: u256_bytes(tx_nonce),
            sender_account_nonce: u256_bytes(sender_nonce),
        }
    }

    fn u256_bytes(value: u64) -> [u8; 32] {
        U256::from(value).to_big_endian()
    }

    fn verify_fact(
        tx_hash: u8,
        chain_id: u64,
        expected_chain_id: u64,
        gas_limit: u64,
        max_gas_limit: u64,
        cornus_active: bool,
        intrinsic_gas_covered: bool,
        signature_valid: bool,
        gas_price: u64,
        minimum_gas_price: u64,
        last_block_number: u64,
    ) -> TransactionManagerVerifyTransactionFact {
        TransactionManagerVerifyTransactionFact {
            tx_hash: [tx_hash; 32],
            chain_id,
            expected_chain_id,
            gas_limit,
            max_gas_limit,
            cornus_active,
            intrinsic_gas_covered,
            signature_valid,
            gas_price: u256_bytes(gas_price),
            minimum_gas_price: u256_bytes(minimum_gas_price),
            last_block_number,
        }
    }

    fn insert_fact(
        tx_hash: u8,
        hash_known: bool,
        queue_status: u8,
        has_finalized_period: bool,
        finalized_period: u64,
    ) -> TransactionManagerInsertTransactionFact {
        TransactionManagerInsertTransactionFact {
            tx_hash: [tx_hash; 32],
            hash_known,
            queue_status,
            has_finalized_period,
            finalized_period,
        }
    }

    fn runtime_queue_input(hash: u8, proposable: bool) -> TransactionQueueInsertInput {
        TransactionQueueInsertInput {
            hash: [hash; 32],
            sender: [9; 20],
            nonce: u256_bytes(1),
            gas_price: u256_bytes(2),
            gas: 21_000,
            data_size: 3,
            tx_rlp: vec![0xaa, 0xbb, 0xcc],
            proposable,
            last_block_number: 0,
        }
    }

    fn runtime_queue_input_for_sender(
        hash: u8,
        sender: [u8; 20],
        nonce: u64,
        proposable: bool,
    ) -> TransactionQueueInsertInput {
        TransactionQueueInsertInput {
            hash: [hash; 32],
            sender,
            nonce: u256_bytes(nonce),
            gas_price: u256_bytes(2),
            gas: 21_000,
            data_size: 3,
            tx_rlp: vec![0xaa, 0xbb, 0xcc],
            proposable,
            last_block_number: 0,
        }
    }

    #[test]
    fn bridge_transaction_manager_verify_transaction_plans_accept_and_reject() {
        assert_eq!(
            transaction_manager_verify_transaction(verify_fact(
                1, 1, 1, 21_000, 100_000, false, true, true, 1, 1, 0
            ))
            .expect("verification plan should compute")
            .status,
            TM_VERIFY_TRANSACTION_STATUS_ACCEPTED
        );
        assert_eq!(
            transaction_manager_verify_transaction(verify_fact(
                1, 2, 1, 21_000, 100_000, false, true, true, 1, 1, 0
            ))
            .expect("verification plan should compute")
            .status,
            TM_VERIFY_TRANSACTION_STATUS_CHAIN_ID_MISMATCH
        );
        assert_eq!(
            transaction_manager_verify_transaction(verify_fact(
                1, 1, 1, 21_000, 100_000, true, false, true, 1, 1, 0
            ))
            .expect("verification plan should compute")
            .status,
            TM_VERIFY_TRANSACTION_STATUS_INTRINSIC_GAS
        );
    }

    #[test]
    fn bridge_transaction_manager_insert_transaction_plans_known_and_finalized() {
        assert_eq!(
            transaction_manager_insert_transaction(insert_fact(
                1,
                true,
                rustaxa_consensus::transaction_queue::TransactionQueueInsertStatus::Inserted as u8,
                false,
                0
            ))
            .expect("insert plan should compute")
            .status,
            TM_INSERT_TRANSACTION_STATUS_ALREADY_KNOWN
        );

        assert_eq!(
            transaction_manager_insert_transaction(insert_fact(
                1,
                false,
                rustaxa_consensus::transaction_queue::TransactionQueueInsertStatus::Known as u8,
                true,
                11
            ))
            .expect("insert plan should compute")
            .status,
            TM_INSERT_TRANSACTION_STATUS_ALREADY_FINALIZED
        );

        assert_eq!(
            transaction_manager_insert_transaction(insert_fact(
                1,
                false,
                rustaxa_consensus::transaction_queue::TransactionQueueInsertStatus::Overflow as u8,
                false,
                0
            ))
            .expect("insert plan should compute")
            .status,
            TM_INSERT_TRANSACTION_STATUS_CANNOT_INSERT
        );
    }

    #[test]
    fn bridge_save_transactions_from_dag_block_command_report_uses_admission_commit_path() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_admission_wrapper");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        storage
            .0
            .metadata()
            .write_status_field(StatusField::TrxCount as u8, 7)
            .expect("status field seed should persist");
        let mut runtime =
            build_transaction_state_from_storage(&storage, TransactionQueueConfig { max_size: 16 })
                .expect("runtime should restore from storage");
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input(1, true))
            .expect("queue seed should succeed");

        let report = save_transactions_from_dag_block_command_report_with_runtime(
            &mut runtime,
            vec![dag_tx_sidecar_fact(0, 1, 5, 4, 0x33)],
        )
        .expect("runtime wrapper should succeed");

        assert_eq!(report.queue_erased.len(), 1);
        assert_eq!(report.queue_erased[0].hash, [1; 32]);
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[1; 32]));
        assert!(runtime.transaction_manager_runtime_contains_non_finalized(&[1; 32]));
        assert_eq!(
            metadata_queries(&storage)
                .get_status_field(StatusField::TrxCount as u8)
                .expect("status field should persist"),
            8
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_update_finalized_transactions_status_command_report_with_runtime_maps_actions() {
        let temp_dir = unique_temp_dir(
            "rustaxa_bridge_tm_update_finalized_status_report_runtime_command_report",
        );
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .0
            .metadata()
            .write_status_field(StatusField::TrxCount as u8, 7)
            .expect("status field seed should persist");

        let mut runtime =
            build_transaction_state_from_storage(&storage, TransactionQueueConfig { max_size: 16 })
                .expect("runtime should restore from storage");
        runtime
            .transaction_manager_runtime_insert_non_finalized(
                TransactionManagerSidecarInsertInput {
                    hash: [1; 32],
                    trx_rlp: vec![0x11],
                },
            )
            .expect("sidecar seed should succeed");
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input(1, true))
            .expect("queue seed should succeed");

        let report = update_finalized_transactions_status_command_report_with_runtime(
            &mut runtime,
            11,
            10,
            vec![FinalizedTransactionStatusSidecarFact {
                input_index: 0,
                hash: [1; 32],
                trx_rlp: vec![0x11],
            }],
        )
        .expect("runtime finalized status command report should execute");

        assert!(!report.purge_transaction_queue);
        assert_eq!(report.removed_non_finalized.len(), 1);
        assert_eq!(report.removed_non_finalized[0].hash, [1; 32]);
        assert_eq!(report.queue_erased.len(), 1);
        assert_eq!(report.queue_erased[0].hash, [1; 32]);
        assert!(report.finalized_account_purged.is_empty());
        assert_eq!(runtime.transaction_manager_runtime_transaction_count(), 7);
        assert_eq!(
            metadata_queries(&storage)
                .get_status_field(StatusField::TrxCount as u8)
                .expect("status field should persist"),
            7
        );
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[1; 32]));
        assert!(runtime.transaction_manager_runtime_contains_recently_finalized(&[1; 32]));
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_update_finalized_transactions_status_command_report_with_runtime_account_nonce_facts_executes_boundary(
    ) {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_update_finalized_status_report_fc_purge");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let sender = [9; 20];
        let _final_chain = crate::final_chain::create_final_chain(
            &storage,
            1_000_000,
            1,
            vec![crate::ffi::rustaxa_ffi::GenesisAccount {
                address: sender,
                balance: ethereum_types::U256::one().to_big_endian().to_vec(),
            }],
            Vec::new(),
            crate::ffi::rustaxa_ffi::GenesisDposConfig {
                eligibility_balance_threshold: vec![1],
                vote_eligibility_balance_step: vec![1],
                validator_maximum_stake: vec![1],
                minimum_deposit: vec![],
                commission_change_delta: 0,
                commission_change_frequency: 0,
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("final chain should initialize");
        storage
            .0
            .metadata()
            .write_status_field(StatusField::TrxCount as u8, 7)
            .expect("status field seed should persist");

        let mut runtime =
            build_transaction_state_from_storage(&storage, TransactionQueueConfig { max_size: 16 })
                .expect("runtime should restore from storage");
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input_for_sender(
                1, sender, 1, true,
            ))
            .expect("queue seed should succeed");

        let report = update_finalized_transactions_status_command_report_with_runtime_and_account_nonce_facts(
                &mut runtime,
                100,
                10,
                vec![crate::ffi::rustaxa_ffi::TransactionQueueAccountNonceFact {
                    sender,
                    account_found: true,
                    account_nonce: U256::from(0u64).to_big_endian(),
                }],
                Vec::new(),
        )
            .expect("runtime finalized status report with final chain should execute purge");

        assert!(!report.purge_transaction_queue);
        assert!(report.removed_non_finalized.is_empty());
        assert!(report.queue_erased.is_empty());
        assert!(report.finalized_account_purged.is_empty());
        assert_eq!(runtime.transaction_manager_runtime_transaction_count(), 7);
        assert!(runtime.transaction_manager_runtime_queue_contains(&[1; 32]));
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_runtime_tracks_multi_account_overflow_drop_window() {
        let mut runtime =
            build_transaction_state_for_test(0, TransactionQueueConfig { max_size: 100 });
        assert!(!runtime.transaction_manager_runtime_queue_transactions_dropped());

        for hash in 1_u8..=101 {
            let outcome = runtime
                .transaction_manager_runtime_queue_insert(runtime_queue_input_for_sender(
                    hash, [hash; 20], 0, true,
                ))
                .expect("multi-account queue insert should succeed");
            if hash <= 100 {
                assert!(outcome.overflow_removed_hashes.is_empty());
            } else {
                assert_eq!(outcome.overflow_removed_hashes.len(), 1);
            }
        }

        assert_eq!(runtime.transaction_manager_runtime_queue_size(), 100);
        assert!(runtime.transaction_manager_runtime_queue_transactions_dropped());
    }

    #[test]
    fn bridge_transaction_manager_runtime_replacement_retains_demoted_payload() {
        let mut runtime =
            build_transaction_state_for_test(0, TransactionQueueConfig { max_size: 100 });
        let original = runtime_queue_input(1, true);
        let original_hash = original.hash;
        let original_rlp = original.tx_rlp.clone();
        runtime
            .transaction_manager_runtime_queue_insert(original)
            .expect("original queue insert should succeed");

        let mut replacement = runtime_queue_input(2, true);
        replacement.gas_price = u256_bytes(3);
        replacement.tx_rlp = vec![0xdd, 0xee];
        let replacement_hash = replacement.hash;
        let replacement_rlp = replacement.tx_rlp.clone();
        let outcome = runtime
            .transaction_manager_runtime_queue_insert(replacement)
            .expect("higher-priced replacement should succeed");

        assert_eq!(outcome.demoted_hashes.len(), 1);
        assert_eq!(outcome.demoted_hashes[0].hash, original_hash);
        assert_eq!(runtime.transaction_manager_runtime_queue_size(), 1);
        assert!(runtime.transaction_manager_runtime_queue_contains(&original_hash));
        assert!(runtime.transaction_manager_runtime_queue_contains(&replacement_hash));
        assert_eq!(
            runtime
                .queue
                .transaction(H256::from(original_hash))
                .expect("demoted transaction payload should remain known")
                .rlp,
            original_rlp
        );
        assert_eq!(
            runtime.queue.ordered_transactions(10)[0].rlp,
            replacement_rlp
        );
    }

    #[test]
    fn runtime_queue_account_nonce_facts_from_bridge_maps_found_and_missing_accounts() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_queue_nonce_facts");
        let facts = runtime_queue_account_nonce_facts_from_bridge(
            vec![H160::from([1; 20]), H160::from([2; 20])],
            vec![
                crate::ffi::rustaxa_ffi::TransactionQueueAccountNonceFact {
                    sender: [1; 20],
                    account_found: true,
                    account_nonce: U256::zero().to_big_endian(),
                },
                crate::ffi::rustaxa_ffi::TransactionQueueAccountNonceFact {
                    sender: [2; 20],
                    account_found: false,
                    account_nonce: U256::zero().to_big_endian(),
                },
            ],
        );

        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].sender, H160::from([1; 20]));
        assert!(facts[0].account_found);
        assert_eq!(facts[0].account_nonce, U256::zero());
        assert_eq!(facts[1].sender, H160::from([2; 20]));
        assert!(!facts[1].account_found);
        assert_eq!(facts[1].account_nonce, U256::zero());

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_runtime_queue_cleanup_with_account_nonce_facts_collects_facts_and_runs(
    ) {
        let sender = [7; 20];
        let mut runtime =
            build_transaction_state_for_test(0, TransactionQueueConfig { max_size: 32 });
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input_for_sender(
                1, sender, 0, true,
            ))
            .expect("proposable nonce=0 insert should succeed");
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input_for_sender(
                2, sender, 1, true,
            ))
            .expect("proposable nonce=1 insert should succeed");
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input_for_sender(
                3, sender, 2, false,
            ))
            .expect("non-proposable nonce=2 insert should succeed");

        let cleanup = runtime
            .transaction_manager_runtime_queue_cleanup_with_account_nonce_facts(
                false,
                20,
                vec![crate::ffi::rustaxa_ffi::TransactionQueueAccountNonceFact {
                    sender,
                    account_found: true,
                    account_nonce: U256::from(0u64).to_big_endian(),
                }],
            )
            .expect("cleanup with account nonce facts should succeed");

        assert!(cleanup.non_proposable_expired.removed_hashes.is_empty());
        assert!(
            cleanup.finalized_account_purged.removed_hashes.len() <= 2,
            "purge should only affect proposable sender entries"
        );
        assert!(runtime.transaction_manager_runtime_queue_contains(&[3; 32]));
    }
}
