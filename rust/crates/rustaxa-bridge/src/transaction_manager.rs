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
use crate::ffi::rustaxa_ffi::TransactionQueueConfig;
use crate::ffi::rustaxa_ffi::{
    DagTransactionSaveSidecarFact, FinalizedTransactionFilterPlan,
    FinalizedTransactionStatusSidecarFact, GasPricerConfig, GasPricerGasPrice,
    TransactionManagerAdmissionCommandReport, TransactionManagerAdmissionResult,
    TransactionManagerAdmissionShellIntent, TransactionManagerDagSaveCommandReport,
    TransactionManagerFilterAction, TransactionManagerFinalChainAdmissionFact,
    TransactionManagerFinalizedStatusCommandReport, TransactionManagerGasEstimationFact,
    TransactionManagerGasEstimationPlan, TransactionManagerGasEstimationResult,
    TransactionManagerHashCommand, TransactionManagerPublicAdmissionCommandReport,
    TransactionManagerPublicInsertResult, TransactionManagerSidecarInsertInput,
    TransactionManagerSidecarLookupRequest, TransactionManagerTransactionView,
    TransactionManagerTransactionViewPlan, TransactionManagerTransactionViewRequest,
    TransactionManagerValidatedInsertRuntimeFact, TransactionManagerVerifyNotFinalizedOutcome,
    TransactionManagerVerifyNotFinalizedSidecarFact, TransactionManagerVerifyTransactionFact,
    TransactionManagerVerifyTransactionOutcome, TransactionPackPreparedPlan,
    TransactionPackSelectedTransaction, TransactionPackSessionCandidate,
    TransactionPackSessionEstimateInput, TransactionPackSessionStep,
    TransactionQueueAccountNonceFact as BridgeTransactionQueueAccountNonceFact,
    TransactionQueueHash, TransactionQueueInsertInput, TransactionQueueStoredTransaction,
    TransactionQueueTransactionGroup,
};
use crate::transaction::legacy_transaction_inspection_from_bytes;
use anyhow::{ensure, Context, Result};
use ethereum_types::{H160, H256, U256};
use rustaxa_consensus::gas_pricer::GasPricerConfig as DomainGasPricerConfig;
use rustaxa_consensus::transaction_manager::{
    plan_exclude_finalized_transactions as plan_exclude_finalized_transactions_from_storage,
    plan_finalized_transactions_status, plan_insert_transaction, plan_transactions_from_dag_block,
    plan_validated_insert, plan_verify_transaction,
    DagTransactionSaveFact as ConsensusDagTransactionSaveFact,
    FinalizedTransactionFilterFact as ConsensusFinalizedTransactionFilterFact,
    FinalizedTransactionFilterPlan as ConsensusFinalizedTransactionFilterPlan,
    FinalizedTransactionStatusFact as ConsensusFinalizedTransactionStatusFact,
    FinalizedTransactionStatusPlan as ConsensusFinalizedTransactionStatusPlan,
    TransactionManagerInsertTransactionFact as ConsensusTransactionManagerInsertTransactionFact,
    TransactionManagerInsertTransactionStatus, TransactionManagerKnownFact,
    TransactionManagerSidecarRecoveryEntry as ConsensusTransactionManagerSidecarRecoveryEntry,
    TransactionManagerValidatedInsertFact as ConsensusTransactionManagerValidatedInsertFact,
    TransactionManagerVerifyTransactionFact as ConsensusTransactionManagerVerifyTransactionFact,
    TransactionManagerVerifyTransactionStatus,
};
use rustaxa_consensus::transaction_packing_service::{
    TransactionPackingCandidate, TransactionPackingEffect, TransactionPackingEstimate,
    TransactionPackingOwner, TransactionPackingRequest, TransactionPackingService,
    TransactionPackingStep,
};
use rustaxa_consensus::transaction_queue::{
    TransactionQueue, TransactionQueueAccountNonceFact, TransactionQueueDemoteStatus,
    TransactionQueueEntry, TransactionQueueInsertStatus, TransactionQueuePurgeOutcome,
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
    append_non_finalized_transactions_to_batch, load_non_finalized_recovery_entries,
    remove_non_finalized_transactions, save_transaction_count, transaction_finalized,
    NonFinalizedTransactionRecoveryEntry, NonFinalizedTransactionStoragePayload,
};
#[cfg(test)]
use rustaxa_storage::StatusField;
use rustaxa_storage::{Storage, StorageWriteBatch};
#[cfg(test)]
use rustaxa_types::FinalChainNonce;
#[cfg(test)]
use rustaxa_types::LegacyTransactionEnvelope;
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

#[cfg(test)]
struct TransactionManagerSidecarHash {
    hash: [u8; 32],
}

#[cfg(test)]
struct TransactionManagerSidecarTransitionInput {
    period: u64,
    hashes: Vec<TransactionManagerSidecarHash>,
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

fn transaction_pack_candidate_from_entry(
    entry: Option<TransactionQueueEntry>,
) -> Result<TransactionPackSessionCandidate> {
    if let Some(entry) = entry {
        let inspection = legacy_transaction_inspection_from_bytes(&entry.rlp, 0)
            .context("TM_RUNTIME_PACK_CANDIDATE_ENVELOPE_INSPECT_FAILED")?;
        ensure!(
            inspection.hash == entry.hash.0,
            "TM_RUNTIME_PACK_CANDIDATE_HASH_MISMATCH"
        );
        ensure!(
            inspection.sender_found && inspection.sender == entry.sender.0,
            "TM_RUNTIME_PACK_CANDIDATE_SENDER_MISMATCH"
        );
        ensure!(
            inspection.nonce == entry.nonce.to_big_endian(),
            "TM_RUNTIME_PACK_CANDIDATE_NONCE_MISMATCH"
        );
        ensure!(
            inspection.gas_price == entry.gas_price.to_big_endian(),
            "TM_RUNTIME_PACK_CANDIDATE_GAS_PRICE_MISMATCH"
        );
        ensure!(
            inspection.gas_limit == entry.gas,
            "TM_RUNTIME_PACK_CANDIDATE_GAS_MISMATCH"
        );
        ensure!(
            inspection.data_size == entry.data_size as usize,
            "TM_RUNTIME_PACK_CANDIDATE_DATA_SIZE_MISMATCH"
        );
        Ok(TransactionPackSessionCandidate {
            found: true,
            hash: entry.hash.0,
            declared_gas: entry.gas,
            sender: entry.sender.0,
            nonce: entry.nonce.to_big_endian(),
            gas_price: entry.gas_price.to_big_endian(),
            gas: entry.gas,
            receiver_found: inspection.receiver_found,
            receiver: inspection.receiver,
            value: inspection.value,
            data: inspection.data,
        })
    } else {
        Ok(TransactionPackSessionCandidate {
            found: false,
            hash: [0; 32],
            declared_gas: 0,
            sender: [0; 20],
            nonce: [0; 32],
            gas_price: [0; 32],
            gas: 0,
            receiver_found: false,
            receiver: [0; 20],
            value: [0; 32],
            data: Vec::new(),
        })
    }
}

fn transaction_pack_session_empty_candidate() -> TransactionPackSessionCandidate {
    TransactionPackSessionCandidate {
        found: false,
        hash: [0; 32],
        declared_gas: 0,
        sender: [0; 20],
        nonce: [0; 32],
        gas_price: [0; 32],
        gas: 0,
        receiver_found: false,
        receiver: [0; 20],
        value: [0; 32],
        data: Vec::new(),
    }
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

/// Filters finalized transactions using Rust runtime sidecars plus storage.
pub fn transaction_manager_filter_non_finalized_with_runtime(
    runtime: &TransactionServiceState,
    requests: Vec<TransactionManagerSidecarLookupRequest>,
) -> Result<FinalizedTransactionFilterPlan> {
    let facts = requests
        .into_iter()
        .map(|request| {
            let hash = H256::from(request.hash);
            ConsensusFinalizedTransactionFilterFact {
                input_index: request.input_index,
                hash,
                in_recently_finalized_cache: runtime.sidecar.contains_recently_finalized(hash),
            }
        })
        .collect();

    let plan: ConsensusFinalizedTransactionFilterPlan =
        plan_exclude_finalized_transactions_from_storage(facts, |hash| {
            transaction_finalized(transaction_manager_runtime_storage(runtime)?, hash)
                .context("TM_FILTER_FINALIZED_LOOKUP")
        })?;

    Ok(FinalizedTransactionFilterPlan {
        not_finalized: plan
            .not_finalized
            .into_iter()
            .map(|action| TransactionManagerFilterAction {
                input_index: action.input_index,
                hash: action.hash.0,
            })
            .collect(),
    })
}

const TM_VERIFY_NOT_FINALIZED_SOURCE_NONE: u8 = 0;
const TM_VERIFY_NOT_FINALIZED_SOURCE_RECENT_SIDECAR: u8 = 1;
const TM_VERIFY_NOT_FINALIZED_SOURCE_STORAGE: u8 = 2;

#[cfg(test)]
fn keccak256(data: &[u8]) -> H256 {
    use tiny_keccak::{Hasher, Keccak};

    let mut output = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(data);
    hasher.finalize(&mut output);
    H256::from(output)
}

/// Verifies transaction hashes against Rust runtime sidecars with sender nonce
/// facts supplied by the external-EVM compatibility boundary.
pub fn transaction_manager_verify_not_finalized_with_runtime(
    runtime: &TransactionServiceState,
    facts: Vec<TransactionManagerVerifyNotFinalizedSidecarFact>,
) -> Result<TransactionManagerVerifyNotFinalizedOutcome> {
    let storage = transaction_manager_runtime_storage(runtime)?;
    for fact in facts {
        let hash = H256::from(fact.hash);
        ensure!(
            !hash.is_zero(),
            "finalized verification transaction hash cannot be zero"
        );
        if runtime.sidecar.contains_recently_finalized(hash) {
            return Ok(TransactionManagerVerifyNotFinalizedOutcome {
                is_finalized: true,
                input_index: fact.input_index,
                hash: hash.0,
                source: TM_VERIFY_NOT_FINALIZED_SOURCE_RECENT_SIDECAR,
            });
        }
        if U256::from_big_endian(&fact.sender_account_nonce)
            >= U256::from_big_endian(&fact.transaction_nonce)
            && transaction_finalized(storage, hash).context("TM_VERIFY_FINALIZED_LOOKUP")?
        {
            return Ok(TransactionManagerVerifyNotFinalizedOutcome {
                is_finalized: true,
                input_index: fact.input_index,
                hash: hash.0,
                source: TM_VERIFY_NOT_FINALIZED_SOURCE_STORAGE,
            });
        }
    }
    Ok(TransactionManagerVerifyNotFinalizedOutcome {
        is_finalized: false,
        input_index: 0,
        hash: [0; 32],
        source: TM_VERIFY_NOT_FINALIZED_SOURCE_NONE,
    })
}

fn transaction_manager_load_nonfinalized_recovery_from_storage(
    storage: &Storage,
) -> Result<Vec<NonFinalizedTransactionRecoveryEntry>> {
    load_non_finalized_recovery_entries(storage).context("TM_NONFINALIZED_RECOVERY_STORAGE")
}

fn transaction_manager_load_nonfinalized_recovery_inputs_from_storage(
    storage: &Storage,
) -> Result<Vec<ConsensusTransactionManagerSidecarRecoveryEntry>> {
    let entries = transaction_manager_load_nonfinalized_recovery_from_storage(storage)?;
    let mut recovered = Vec::with_capacity(entries.len());

    for entry in entries {
        if entry.finalized {
            continue;
        }

        let inspection = legacy_transaction_inspection_from_bytes(&entry.trx_rlp, 0)
            .context("TM_NONFINALIZED_RECOVERY_ENVELOPE_INSPECT")?;
        ensure!(
            H256::from(inspection.hash) == entry.hash,
            "TM_NONFINALIZED_RECOVERY_HASH_MISMATCH"
        );
        ensure!(
            inspection.sender_found,
            "TM_NONFINALIZED_RECOVERY_SENDER_MISSING"
        );

        recovered.push(ConsensusTransactionManagerSidecarRecoveryEntry {
            hash: entry.hash,
            finalized: false,
            trx_rlp: inspection.tx_rlp,
        });
    }

    Ok(recovered)
}

/// Rebuilds runtime recovery sidecars from Rust-backed storage without exposing count mirrors.
pub fn transaction_manager_recover_nonfinalized_with_runtime(
    runtime: &mut TransactionServiceState,
) -> Result<()> {
    let entries = transaction_manager_load_nonfinalized_recovery_inputs_from_storage(
        transaction_manager_runtime_storage(runtime)?,
    )?;
    TransactionRuntimeAccess(runtime)
        .insert_recovery_entries(entries)
        .map(|_| ())
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
    /// Begins one runtime-owned packing flow and returns all candidates that
    /// require C++ gas estimation.
    #[allow(clippy::too_many_arguments)]
    pub fn transaction_manager_runtime_pack_prepare_sharded(
        &mut self,
        weight_limit: u64,
        min_transaction_gas: u64,
        proposal_period: u64,
        estimate_gas_limit: u64,
        last_block_number: u64,
        total_shards: u16,
        node_shard: u16,
        shard_period_interval: u64,
    ) -> Result<TransactionPackPreparedPlan> {
        self.transaction_manager_runtime_pack_prepare_sharded_for_owner(
            TransactionPackingOwner::Compatibility,
            weight_limit,
            min_transaction_gas,
            proposal_period,
            estimate_gas_limit,
            last_block_number,
            total_shards,
            node_shard,
            shard_period_interval,
        )
    }

    /// Begins packing for a validated internal owner.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn transaction_manager_runtime_pack_prepare_sharded_for_owner(
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
    ) -> Result<TransactionPackPreparedPlan> {
        let candidate_limit = TransactionPackingService::candidate_limit(
            weight_limit,
            min_transaction_gas,
            total_shards,
            node_shard,
            shard_period_interval,
        )?;
        let candidates = self
            .0
            .queue
            .ordered_transactions(candidate_limit)
            .into_iter()
            .map(|entry| {
                let cached_gas_used = self
                    .0
                    .sidecar
                    .gas_estimation_cache_get(entry.hash, proposal_period)
                    .context("TM_RUNTIME_PACK_GAS_ESTIMATION_CACHE_GET")?
                    .map(|cached| cached.gas_used);
                Ok(TransactionPackingCandidate {
                    entry,
                    cached_gas_used,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let step = self
            .0
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
                candidates,
            })
            .context("TM_RUNTIME_PACK_PREPARE")?;
        let result = (|| {
            let (selected_transactions, mut demoted_hashes) =
                self.apply_packing_step_effects(&step)?;
            if !step.request_estimates.is_empty() {
                self.transaction_packing.acknowledge_demotions(
                    owner,
                    demoted_hashes
                        .iter()
                        .map(|hash| H256::from(hash.hash))
                        .collect(),
                )?;
            }
            demoted_hashes.splice(
                0..0,
                runtime_hashes_to_bridge(step.acknowledged_demotions.clone()),
            );
            Ok(TransactionPackPreparedPlan {
                request_estimates: step
                    .request_estimates
                    .iter()
                    .cloned()
                    .map(|candidate| transaction_pack_candidate_from_entry(Some(candidate.entry)))
                    .collect::<Result<Vec<_>>>()?,
                selected_transactions,
                demoted_hashes,
                stopped: step.stopped,
            })
        })();
        if result.is_err() {
            let _ = self.transaction_packing.abort(owner);
        }
        result
    }

    /// Completes one prepared packing plan with a batch of C++ estimates.
    ///
    /// Rust applies estimates in-order and returns final selected transactions
    /// and demotion facts as one deterministic output.
    pub fn transaction_manager_runtime_pack_finalize_with_estimates(
        &mut self,
        inputs: Vec<TransactionPackSessionEstimateInput>,
    ) -> Result<TransactionPackSessionStep> {
        self.transaction_manager_runtime_pack_finalize_with_estimates_for_owner(
            TransactionPackingOwner::Compatibility,
            inputs,
        )
    }

    /// Completes packing only when the active cursor belongs to `owner`.
    pub(crate) fn transaction_manager_runtime_pack_finalize_with_estimates_for_owner(
        &mut self,
        owner: TransactionPackingOwner,
        inputs: Vec<TransactionPackSessionEstimateInput>,
    ) -> Result<TransactionPackSessionStep> {
        let step = self.transaction_packing.finalize(
            owner,
            inputs
                .into_iter()
                .map(|input| TransactionPackingEstimate {
                    hash: H256::from(input.hash),
                    gas_used: input.gas_used,
                    last_block_number: input.last_block_number,
                    result_rlp: input.result_rlp,
                })
                .collect(),
        )?;
        let (selected_transactions, mut demoted_hashes) = self.apply_packing_step_effects(&step)?;
        demoted_hashes.splice(
            0..0,
            runtime_hashes_to_bridge(step.acknowledged_demotions.clone()),
        );
        Ok(TransactionPackSessionStep {
            request_estimate: false,
            candidate: transaction_pack_session_empty_candidate(),
            selected_transactions,
            demoted_hashes,
            stopped: step.stopped,
        })
    }

    /// Aborts and clears the active runtime packing session.
    pub fn transaction_manager_runtime_pack_abort(&mut self) -> bool {
        self.transaction_manager_runtime_pack_abort_for_owner(
            TransactionPackingOwner::Compatibility,
        )
    }

    /// Aborts only a packing cursor owned by `owner`.
    pub(crate) fn transaction_manager_runtime_pack_abort_for_owner(
        &mut self,
        owner: TransactionPackingOwner,
    ) -> bool {
        self.transaction_packing
            .abort(owner)
            .expect("TM_RUNTIME_PACKING_LOCK_POISONED")
    }

    fn apply_packing_step_effects(
        &mut self,
        step: &TransactionPackingStep,
    ) -> Result<(
        Vec<TransactionPackSelectedTransaction>,
        Vec<TransactionQueueHash>,
    )> {
        let mut demoted_hashes = Vec::new();
        for effect in &step.effects {
            match effect {
                TransactionPackingEffect::Demote(intent) => {
                    let outcome = self.queue.demote(intent.hash, intent.last_block_number);
                    if matches!(outcome.status, TransactionQueueDemoteStatus::Demoted) {
                        demoted_hashes.push(TransactionQueueHash {
                            hash: intent.hash.0,
                        });
                    }
                }
                TransactionPackingEffect::CacheInsert(intent) => {
                    self.sidecar
                        .gas_estimation_cache_insert(
                            intent.hash,
                            intent.proposal_period,
                            intent.gas_used,
                            intent.result_rlp.clone(),
                        )
                        .context("TM_RUNTIME_PACK_GAS_ESTIMATION_CACHE_STORE")?;
                }
            }
        }
        Ok((
            step.selected
                .iter()
                .map(|selected| TransactionPackSelectedTransaction {
                    hash: selected.hash.0,
                    gas_used: selected.gas_used,
                    tx_rlp: selected.transaction_rlp.clone(),
                })
                .collect(),
            demoted_hashes,
        ))
    }

    /// Stores one opaque C++ `ExecutionResult` RLP in the Rust-owned estimation cache.
    pub fn transaction_manager_runtime_store_gas_estimation(
        &mut self,
        result: TransactionManagerGasEstimationResult,
    ) -> Result<bool> {
        self.sidecar
            .gas_estimation_cache_insert(
                H256::from(result.hash),
                result.proposal_period,
                result.gas_used,
                result.result_rlp,
            )
            .context("TM_RUNTIME_GAS_ESTIMATION_CACHE_STORE")
    }

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

    /// Inserts finalized payloads and transitions them to recently-finalized sidecar state.
    ///
    /// C++ supplies only canonical hash/RLP payload facts extracted at the
    /// compatibility edge. Rust performs the full sidecar mutation sequence so
    /// finalized-sidecar initialization is not orchestrated by repeated C++
    /// calls.
    pub fn transaction_manager_runtime_initialize_recently_finalized_payloads(
        &mut self,
        period: u64,
        payloads: Vec<TransactionManagerSidecarInsertInput>,
    ) -> Result<()> {
        let mut hashes = Vec::with_capacity(payloads.len());
        for input in payloads {
            let hash = H256::from(input.hash);
            self.sidecar
                .insert_non_finalized(hash, input.trx_rlp)
                .context("TM_RUNTIME_RECENT_FINALIZED_INIT_INSERT")?;
            hashes.push(hash);
        }
        self.sidecar
            .apply_finalized_transition(period, hashes)
            .context("TM_RUNTIME_RECENT_FINALIZED_INIT_TRANSITION")
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

    /// Removes requested non-finalized sidecar payloads and returns the removal count.
    pub fn transaction_manager_runtime_remove_non_finalized(
        &mut self,
        requests: Vec<TransactionManagerSidecarLookupRequest>,
    ) -> Result<u64> {
        let mut hashes = Vec::with_capacity(requests.len());
        for request in &requests {
            let hash = H256::from(request.hash);
            ensure!(
                !hash.is_zero(),
                "runtime sidecar removal hash cannot be zero"
            );
            if self.sidecar.contains_non_finalized(hash) {
                hashes.push(hash);
            }
        }

        remove_non_finalized_transactions(
            transaction_manager_runtime_storage(self)?,
            hashes.clone(),
        )
        .context("TM_RUNTIME_REMOVE_NON_FINALIZED_STORAGE")?;

        let mut removed = 0u64;
        for hash in hashes {
            if self.sidecar.remove_non_finalized(hash) {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Moves finalized hashes from non-finalized to recently-finalized sidecar state.
    #[cfg(test)]
    fn transaction_manager_runtime_apply_finalized_transition(
        &mut self,
        transition: TransactionManagerSidecarTransitionInput,
    ) -> Result<()> {
        self.sidecar
            .apply_finalized_transition(
                transition.period,
                transition
                    .hashes
                    .into_iter()
                    .map(|hash| H256::from(hash.hash))
                    .collect::<Vec<_>>(),
            )
            .context("TM_RUNTIME_FINALIZED_TRANSITION")
    }

    fn insert_recovery_entries(
        &mut self,
        entries: Vec<ConsensusTransactionManagerSidecarRecoveryEntry>,
    ) -> Result<u64> {
        Ok(self
            .0
            .sidecar
            .insert_recovery_entries(entries)
            .context("TM_RUNTIME_RECOVERY_INSERT")? as u64)
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

    /// Applies finalized-block expiry to non-proposable queue state.
    pub fn transaction_manager_runtime_queue_block_finalized(
        &mut self,
        block_number: u64,
    ) -> Vec<TransactionQueueHash> {
        runtime_hashes_to_bridge(self.queue.block_finalized(block_number))
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

    /// Updates finalized-block gas-price history from canonical price facts.
    ///
    /// Empty inputs and pool-mode runtimes are deliberate no-ops, preserving
    /// native oracle behavior. The update owns no storage write.
    pub fn transaction_manager_runtime_gas_price_update(
        &mut self,
        gas_prices: Vec<GasPricerGasPrice>,
    ) {
        self.gas_price_oracle.update_from_gas_prices(
            gas_prices
                .into_iter()
                .map(|gas_price| U256::from_big_endian(&gas_price.price)),
        );
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
    use crate::ffi::{
        BridgeFinalChain, BridgeMetadataStorageQueries, BridgeStorage,
        BridgeTransactionStorageQueries,
    };
    use crate::storage::{create_metadata_storage_queries, create_transaction_storage_queries};
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
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

    fn transaction_queries(storage: &BridgeStorage) -> Box<BridgeTransactionStorageQueries> {
        create_transaction_storage_queries(storage)
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

    fn address_from_signing_key(signing_key: &SigningKey) -> H160 {
        let public_key = signing_key.verifying_key().to_encoded_point(false);
        let public_key_hash = keccak256(&public_key.as_bytes()[1..]);
        H160::from_slice(&public_key_hash.as_bytes()[12..])
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

    fn signed_pbft_block(signing_key: &SigningKey, period: u64, timestamp: u64) -> Vec<u8> {
        let mut unsigned_stream = RlpStream::new_list(7);
        append_pbft_block_fields(&mut unsigned_stream, period, timestamp);
        let message_hash = keccak256(&unsigned_stream.out());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(message_hash.as_bytes())
            .expect("test PBFT block should sign");
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(recovery_id.to_byte());

        let mut signed_stream = RlpStream::new_list(8);
        append_pbft_block_fields(&mut signed_stream, period, timestamp);
        signed_stream.append(&signature_bytes);
        signed_stream.out().to_vec()
    }

    fn period_data_rlp_with_pbft(pbft_block_rlp: &[u8], transaction_rlps: &[Vec<u8>]) -> Vec<u8> {
        let mut transactions = RlpStream::new_list(transaction_rlps.len());
        for transaction_rlp in transaction_rlps {
            transactions.append_raw(transaction_rlp, 1);
        }

        let mut period_data = RlpStream::new_list(5);
        period_data.append_raw(pbft_block_rlp, 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&transactions.out(), 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.out().to_vec()
    }

    fn signed_legacy_transaction_rlp(
        signing_key: &SigningKey,
        nonce: u64,
        chain_id: u64,
    ) -> Vec<u8> {
        let mut signature_stream = RlpStream::new_list(9);
        signature_stream.append(&U256::from(nonce));
        signature_stream.append(&U256::from(2u64));
        signature_stream.append(&21_000u64);
        signature_stream.append(&H160::from([0x44u8; 20]));
        signature_stream.append(&U256::from(3u64));
        signature_stream.append(&Vec::<u8>::new());
        signature_stream.append(&U256::from(chain_id));
        signature_stream.append(&U256::zero());
        signature_stream.append(&U256::zero());
        let message_hash = keccak256(&signature_stream.out());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(message_hash.as_bytes())
            .expect("test transaction should sign");
        let signature = signature.to_bytes();
        let r = U256::from_big_endian(&signature[..32]);
        let s = U256::from_big_endian(&signature[32..]);
        let v = U256::from(chain_id * 2 + 35 + u64::from(recovery_id.to_byte()));

        let mut stream = RlpStream::new_list(9);
        stream.append(&U256::from(nonce));
        stream.append(&U256::from(2u64));
        stream.append(&21_000u64);
        stream.append(&H160::from([0x44u8; 20]));
        stream.append(&U256::from(3u64));
        stream.append(&Vec::<u8>::new());
        stream.append(&v);
        stream.append(&r);
        stream.append(&s);
        stream.out().to_vec()
    }

    fn finalize_regular_transaction_for_test(
        final_chain: &BridgeFinalChain,
        pbft_block_rlp: Vec<u8>,
        transaction: crate::ffi::rustaxa_ffi::FinalizationTransaction,
    ) {
        final_chain
            .0
            .finalize_block(
                pbft_block_rlp,
                vec![rustaxa_consensus::FinalizationTransaction {
                    hash: transaction.hash,
                    sender: transaction.sender,
                    receiver: if transaction.receiver_found {
                        Some(transaction.receiver)
                    } else {
                        None
                    },
                    nonce: FinalChainNonce::from_bytes(&transaction.nonce)
                        .expect("transaction-manager test nonce should be canonical"),
                    value: rustaxa_types::FinalChainTransactionValue::try_from(
                        transaction.value.as_slice(),
                    )
                    .expect("transaction-manager test value should fit u256"),
                    gas_price: rustaxa_types::FinalChainGasPrice::try_from(
                        transaction.gas_price.as_slice(),
                    )
                    .expect("transaction-manager test gas price should fit u256"),
                    gas_limit: transaction.gas_limit.into(),
                    data: transaction.data,
                    rlp: transaction.rlp,
                }],
                Vec::new(),
            )
            .expect("finalization should create block-scoped account snapshot");
    }

    #[test]
    fn bridge_transaction_manager_runtime_remove_non_finalized_deletes_storage_rows() {
        let temp_dir =
            unique_temp_dir("rustaxa_bridge_transaction_manager_runtime_remove_non_finalized");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let mut runtime =
            build_transaction_state_from_storage(&storage, TransactionQueueConfig { max_size: 16 })
                .expect("runtime should restore from storage");

        for (hash, trx_rlp) in [
            ([1u8; 32], vec![0x11]),
            ([2u8; 32], vec![0x22]),
            ([3u8; 32], vec![0x33]),
        ] {
            storage
                .0
                .transaction()
                .write(H256::from(hash), &trx_rlp)
                .expect("pending transaction should persist");
        }
        for (hash, trx_rlp) in [([1u8; 32], vec![0x11]), ([2u8; 32], vec![0x22])] {
            runtime
                .transaction_manager_runtime_insert_non_finalized(
                    TransactionManagerSidecarInsertInput { hash, trx_rlp },
                )
                .expect("sidecar payload should insert");
        }

        let removed = runtime
            .transaction_manager_runtime_remove_non_finalized(vec![
                TransactionManagerSidecarLookupRequest {
                    input_index: 0,
                    hash: [1u8; 32],
                },
                TransactionManagerSidecarLookupRequest {
                    input_index: 1,
                    hash: [3u8; 32],
                },
            ])
            .expect("runtime removal should delete storage-backed sidecar rows");

        assert_eq!(removed, 1);
        assert!(!runtime.transaction_manager_runtime_contains_non_finalized(&[1u8; 32]));
        assert!(runtime.transaction_manager_runtime_contains_non_finalized(&[2u8; 32]));
        assert_eq!(
            storage.0.transaction().rlp(H256::from([1u8; 32])).unwrap(),
            None
        );
        assert_eq!(
            storage.0.transaction().rlp(H256::from([2u8; 32])).unwrap(),
            Some(vec![0x22])
        );
        assert_eq!(
            storage.0.transaction().rlp(H256::from([3u8; 32])).unwrap(),
            Some(vec![0x33])
        );

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
    fn bridge_transaction_manager_filter_non_finalized_with_runtime_uses_live_sidecar_and_storage()
    {
        let temp_dir =
            unique_temp_dir("rustaxa_bridge_transaction_manager_runtime_filter_non_finalized");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let mut runtime =
            build_transaction_state_from_storage(&storage, TransactionQueueConfig { max_size: 16 })
                .expect("runtime should restore from storage");

        storage
            .0
            .transaction()
            .write_location(H256::from([2u8; 32]), 7, 0, false)
            .expect("finalized hash should be persisted in trx period");
        runtime
            .transaction_manager_runtime_insert_non_finalized(
                TransactionManagerSidecarInsertInput {
                    hash: [3; 32],
                    trx_rlp: vec![0x03],
                },
            )
            .expect("runtime sidecar insert should succeed");
        runtime
            .transaction_manager_runtime_apply_finalized_transition(
                TransactionManagerSidecarTransitionInput {
                    period: 7,
                    hashes: vec![TransactionManagerSidecarHash { hash: [3; 32] }],
                },
            )
            .expect("runtime finalized transition should succeed");

        let out = transaction_manager_filter_non_finalized_with_runtime(
            &runtime,
            vec![
                TransactionManagerSidecarLookupRequest {
                    input_index: 0,
                    hash: [1; 32],
                },
                TransactionManagerSidecarLookupRequest {
                    input_index: 1,
                    hash: [2; 32],
                },
                TransactionManagerSidecarLookupRequest {
                    input_index: 2,
                    hash: [3; 32],
                },
            ],
        )
        .expect("runtime filtering plan should map finalized inputs");

        assert_eq!(out.not_finalized.len(), 1);
        assert_eq!(out.not_finalized[0].input_index, 0);
        assert_eq!(out.not_finalized[0].hash, [1; 32]);

        let _ = fs::remove_dir_all(temp_dir);
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
    fn bridge_transaction_manager_recovery_payloads_mark_stale_finalized_entries() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_transaction_manager_recovery_payloads");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .0
            .transaction()
            .write(H256::from([1u8; 32]), &[0x11])
            .expect("non-finalized transaction should persist");
        storage
            .0
            .transaction()
            .write(H256::from([2u8; 32]), &[0x22])
            .expect("finalized stale entry should persist");
        storage
            .0
            .transaction()
            .write_location(H256::from([2u8; 32]), 11, 0, false)
            .expect("stale finalized entry location should persist");

        let mut txs = RlpStream::new_list(1);
        txs.append_raw(&[0x22], 1);

        let mut period_data = RlpStream::new_list(5);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&txs.out(), 1);
        period_data.append_raw(&[0xC0], 1);
        storage
            .0
            .period()
            .write(11, &period_data.out().as_ref().to_vec())
            .expect("period data should persist");

        let out = transaction_manager_load_nonfinalized_recovery_from_storage(&storage.0)
            .expect("recovery payload lookup should inspect all non-finalized storage rows");

        assert_eq!(out.len(), 2);
        let mut by_hash = out
            .into_iter()
            .map(|entry| (entry.hash.0[0], entry.finalized))
            .collect::<Vec<_>>();
        by_hash.sort_unstable();
        assert_eq!(by_hash, vec![(1u8, false), (2u8, true)]);
        assert_eq!(
            transaction_queries(&storage)
                .get_transaction(&[2u8; 32])
                .expect("stale finalized entry should be removed"),
            Vec::<u8>::new()
        );
        assert_eq!(
            transaction_queries(&storage)
                .get_transaction(&[1u8; 32])
                .expect("live non-finalized entry should remain"),
            vec![0x11]
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_recovery_inputs_validate_survivor_envelopes() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_transaction_manager_recovery_inputs");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        let signing_key = SigningKey::from_slice(&[0x43u8; 32]).unwrap();
        let tx_rlp = signed_legacy_transaction_rlp(&signing_key, 1, 2999);
        let envelope = LegacyTransactionEnvelope::decode(&tx_rlp).unwrap();
        storage
            .0
            .transaction()
            .write(envelope.hash, &tx_rlp)
            .expect("non-finalized transaction should persist");

        let inputs = transaction_manager_load_nonfinalized_recovery_inputs_from_storage(&storage.0)
            .expect("recovery inputs should validate live survivor envelopes");

        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].hash, envelope.hash);
        assert!(!inputs[0].finalized);
        assert_eq!(inputs[0].trx_rlp, tx_rlp);

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_recover_nonfinalized_command_report_inserts_survivors() {
        let temp_dir =
            unique_temp_dir("rustaxa_bridge_transaction_manager_recover_nonfinalized_cmd_report");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        let signing_key = SigningKey::from_slice(&[0x43u8; 32]).unwrap();
        let live_tx = signed_legacy_transaction_rlp(&signing_key, 1, 2999);
        let live_hash = LegacyTransactionEnvelope::decode(&live_tx)
            .expect("live transaction should decode")
            .hash
            .0;
        storage
            .0
            .transaction()
            .write(H256::from(live_hash), &live_tx)
            .expect("non-finalized transaction should persist");
        storage
            .0
            .transaction()
            .write(H256::from([2u8; 32]), &[0x22])
            .expect("stale transaction should persist");
        storage
            .0
            .transaction()
            .write_location(H256::from([2u8; 32]), 11, 0, false)
            .expect("stale finalized location should persist");
        storage
            .0
            .metadata()
            .write_status_field(StatusField::TrxCount as u8, 4)
            .expect("status field seed should persist");

        let mut runtime =
            build_transaction_state_from_storage(&storage, TransactionQueueConfig { max_size: 16 })
                .expect("runtime should restore from storage");

        transaction_manager_recover_nonfinalized_with_runtime(&mut runtime)
            .expect("runtime recovery should execute");

        assert_eq!(runtime.transaction_manager_runtime_transaction_count(), 4);
        assert!(runtime.transaction_manager_runtime_contains_non_finalized(&live_hash));
        assert_eq!(
            transaction_queries(&storage)
                .get_transaction(&[2u8; 32])
                .expect("stale tx should be removed"),
            Vec::<u8>::new()
        );
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
    fn bridge_transaction_manager_runtime_queue_block_finalized_returns_expired_hashes() {
        let mut runtime =
            build_transaction_state_for_test(0, TransactionQueueConfig { max_size: 16 });
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input(1, false))
            .expect("non-proposable insert should succeed");
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input(2, false))
            .expect("non-proposable insert should succeed");

        let expired = runtime.transaction_manager_runtime_queue_block_finalized(20);

        assert_eq!(runtime.transaction_manager_runtime_transaction_count(), 0);
        assert_eq!(expired.len(), 2);
        assert_ne!(expired[0].hash, expired[1].hash);
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[1; 32]));
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[2; 32]));
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

    #[test]
    fn bridge_transaction_manager_runtime_pack_prepare_finalize_single_candidate() {
        let mut runtime =
            build_transaction_state_for_test(0, TransactionQueueConfig { max_size: 8 });
        let signing_key = SigningKey::from_slice(&[0x47u8; 32]).unwrap();
        let sender = address_from_signing_key(&signing_key);
        let tx_rlp = signed_legacy_transaction_rlp(&signing_key, 1, 2999);
        let envelope = LegacyTransactionEnvelope::decode(&tx_rlp).unwrap();
        runtime
            .transaction_manager_runtime_queue_insert(TransactionQueueInsertInput {
                hash: envelope.hash.0,
                sender: sender.0,
                nonce: envelope.nonce.to_big_endian(),
                gas_price: envelope.gas_price.to_big_endian(),
                gas: envelope.gas,
                data_size: envelope.data.len(),
                tx_rlp: tx_rlp,
                proposable: true,
                last_block_number: 0,
            })
            .expect("queue insert should succeed");

        let plan = runtime
            .transaction_manager_runtime_pack_prepare_sharded(63_000, 21_000, 7, 0, 10, 1, 0, 1)
            .expect("pack prepare should return estimate plan");
        assert_eq!(plan.request_estimates.len(), 1);
        assert_eq!(plan.request_estimates[0].hash, envelope.hash.0);

        let estimate = vec![TransactionPackSessionEstimateInput {
            hash: envelope.hash.0,
            gas_used: 42_000,
            last_block_number: 10,
            result_rlp: vec![0xc0],
        }];
        let final_step = runtime
            .transaction_manager_runtime_pack_finalize_with_estimates(estimate)
            .expect("pack finalize should return selected set");

        assert!(!final_step.request_estimate);
        assert!(final_step.stopped);
        assert_eq!(final_step.selected_transactions.len(), 1);
        assert_eq!(final_step.selected_transactions[0].hash, envelope.hash.0);
        assert_eq!(final_step.selected_transactions[0].gas_used, 42_000);
        assert!(!runtime.transaction_manager_runtime_pack_abort());
    }

    #[test]
    fn bridge_transaction_manager_runtime_pack_prepare_conversion_error_aborts_session() {
        let mut runtime =
            build_transaction_state_for_test(0, TransactionQueueConfig { max_size: 8 });
        let signing_key = SigningKey::from_slice(&[0x59u8; 32]).unwrap();
        let sender = address_from_signing_key(&signing_key);
        let valid_rlp = signed_legacy_transaction_rlp(&signing_key, 1, 2999);
        let envelope = LegacyTransactionEnvelope::decode(&valid_rlp).unwrap();
        runtime
            .transaction_manager_runtime_queue_insert(TransactionQueueInsertInput {
                hash: envelope.hash.0,
                sender: sender.0,
                nonce: envelope.nonce.to_big_endian(),
                gas_price: envelope.gas_price.to_big_endian(),
                gas: envelope.gas,
                data_size: envelope.data.len(),
                tx_rlp: vec![0xff],
                proposable: true,
                last_block_number: 0,
            })
            .expect("queue insert should retain the supplied canonical-payload fact");

        let error = runtime
            .transaction_manager_runtime_pack_prepare_sharded(63_000, 21_000, 7, 0, 10, 1, 0, 1)
            .err()
            .expect("malformed retained candidate payload must fail conversion");
        assert!(error
            .to_string()
            .contains("TM_RUNTIME_PACK_CANDIDATE_ENVELOPE_INSPECT_FAILED"));
        assert!(!runtime.transaction_packing.is_active().unwrap());
        assert!(!runtime.transaction_manager_runtime_pack_abort());
    }

    #[test]
    fn bridge_transaction_manager_runtime_pack_enforces_owner_on_finalize_and_abort() {
        let mut runtime =
            build_transaction_state_for_test(0, TransactionQueueConfig { max_size: 8 });
        let signing_key = SigningKey::from_slice(&[0x57u8; 32]).unwrap();
        let sender = address_from_signing_key(&signing_key);
        let tx_rlp = signed_legacy_transaction_rlp(&signing_key, 1, 2999);
        let envelope = LegacyTransactionEnvelope::decode(&tx_rlp).unwrap();
        runtime
            .transaction_manager_runtime_queue_insert(TransactionQueueInsertInput {
                hash: envelope.hash.0,
                sender: sender.0,
                nonce: envelope.nonce.to_big_endian(),
                gas_price: envelope.gas_price.to_big_endian(),
                gas: envelope.gas,
                data_size: envelope.data.len(),
                tx_rlp,
                proposable: true,
                last_block_number: 0,
            })
            .expect("queue insert");

        let owner = TransactionPackingOwner::DagProposer(42);
        let plan = runtime
            .transaction_manager_runtime_pack_prepare_sharded_for_owner(
                owner, 63_000, 21_000, 7, 0, 10, 1, 0, 1,
            )
            .expect("owner pack prepare");
        assert_eq!(plan.request_estimates.len(), 1);
        let error = runtime
            .transaction_manager_runtime_pack_finalize_with_estimates(vec![])
            .err()
            .expect("wrong owner must fail");
        assert!(error
            .to_string()
            .contains("TM_RUNTIME_PACK_SESSION_OWNER_MISMATCH"));
        assert!(!runtime.transaction_manager_runtime_pack_abort());
        assert!(runtime.transaction_packing.is_active().unwrap());
        assert!(!runtime.transaction_manager_runtime_pack_abort_for_owner(
            TransactionPackingOwner::DagProposer(41)
        ));
        assert!(runtime.transaction_manager_runtime_pack_abort_for_owner(owner));
        assert!(!runtime.transaction_packing.is_active().unwrap());
    }

    #[test]
    fn bridge_transaction_manager_runtime_pack_prepare_with_declared_gas_selected() {
        let mut runtime =
            build_transaction_state_for_test(0, TransactionQueueConfig { max_size: 8 });
        let signing_key = SigningKey::from_slice(&[0x48u8; 32]).unwrap();
        let sender = address_from_signing_key(&signing_key);
        let tx_rlp = signed_legacy_transaction_rlp(&signing_key, 1, 2999);
        let envelope = LegacyTransactionEnvelope::decode(&tx_rlp).unwrap();
        runtime
            .transaction_manager_runtime_queue_insert(TransactionQueueInsertInput {
                hash: envelope.hash.0,
                sender: sender.0,
                nonce: envelope.nonce.to_big_endian(),
                gas_price: envelope.gas_price.to_big_endian(),
                gas: envelope.gas,
                data_size: envelope.data.len(),
                tx_rlp,
                proposable: true,
                last_block_number: 0,
            })
            .expect("queue insert should succeed");

        let plan = runtime
            .transaction_manager_runtime_pack_prepare_sharded(
                63_000, 21_000, 7, 50_000, 10, 1, 0, 1,
            )
            .expect("pack prepare should prefer declared-gas path");
        assert!(plan.request_estimates.is_empty());
        let final_step = TransactionPackSessionStep {
            request_estimate: false,
            candidate: transaction_pack_session_empty_candidate(),
            selected_transactions: plan.selected_transactions,
            demoted_hashes: plan.demoted_hashes,
            stopped: plan.stopped,
        };
        assert_eq!(final_step.selected_transactions.len(), 1);
        assert_eq!(final_step.selected_transactions[0].gas_used, envelope.gas);
        assert!(!final_step.request_estimate);
    }

    #[test]
    fn bridge_transaction_manager_runtime_pack_prepare_filters_candidate_shards() {
        fn legacy_sender_shard(sender: H160, proposal_period: u64, total_shards: u16) -> u16 {
            let prefix = u64::from_be_bytes([
                0,
                0,
                0,
                sender.0[0],
                sender.0[1],
                sender.0[2],
                sender.0[3],
                sender.0[4],
            ]);
            ((prefix + proposal_period / 10) % u64::from(total_shards)) as u16
        }

        let proposal_period = 27;
        let total_shards = 2;
        let first_key = SigningKey::from_slice(&[0x45u8; 32]).unwrap();
        let first_sender = address_from_signing_key(&first_key);
        let first_shard = legacy_sender_shard(first_sender, proposal_period, total_shards);
        let (second_key, second_sender) = (0x46u8..=0x7f)
            .map(|seed| {
                let key = SigningKey::from_slice(&[seed; 32]).unwrap();
                let sender = address_from_signing_key(&key);
                (key, sender)
            })
            .find(|(_, sender)| {
                legacy_sender_shard(*sender, proposal_period, total_shards) != first_shard
            })
            .expect("test fixture should find a sender in a different shard");

        let mut runtime =
            build_transaction_state_for_test(0, TransactionQueueConfig { max_size: 8 });
        let first_rlp = signed_legacy_transaction_rlp(&first_key, 1, 2999);
        let first_envelope = LegacyTransactionEnvelope::decode(&first_rlp).unwrap();
        let second_rlp = signed_legacy_transaction_rlp(&second_key, 1, 2999);
        let second_envelope = LegacyTransactionEnvelope::decode(&second_rlp).unwrap();

        for (sender, envelope, tx_rlp) in [
            (first_sender, first_envelope.clone(), first_rlp),
            (second_sender, second_envelope.clone(), second_rlp),
        ] {
            runtime
                .transaction_manager_runtime_queue_insert(TransactionQueueInsertInput {
                    hash: envelope.hash.0,
                    sender: sender.0,
                    nonce: envelope.nonce.to_big_endian(),
                    gas_price: envelope.gas_price.to_big_endian(),
                    gas: envelope.gas,
                    data_size: envelope.data.len(),
                    tx_rlp,
                    proposable: true,
                    last_block_number: 0,
                })
                .expect("proposable insert should succeed");
        }

        let plan = runtime
            .transaction_manager_runtime_pack_prepare_sharded(
                63_000,
                21_000,
                proposal_period,
                0,
                10,
                total_shards,
                first_shard,
                10,
            )
            .expect("sharded pack prepare should begin");
        assert_eq!(plan.request_estimates.len(), 1);
        assert_eq!(plan.request_estimates[0].hash, first_envelope.hash.0);

        let final_step = runtime
            .transaction_manager_runtime_pack_finalize_with_estimates(vec![
                TransactionPackSessionEstimateInput {
                    hash: plan.request_estimates[0].hash,
                    gas_used: 30_000,
                    last_block_number: 10,
                    result_rlp: vec![0xc0],
                },
            ])
            .expect("estimate should finish sharded pack session");
        assert!(!final_step.request_estimate);
        assert_eq!(final_step.selected_transactions.len(), 1);
        assert_eq!(
            final_step.selected_transactions[0].hash,
            first_envelope.hash.0
        );
        assert_ne!(
            final_step.selected_transactions[0].hash,
            second_envelope.hash.0
        );
        assert!(!runtime.transaction_manager_runtime_pack_abort());
    }

    #[test]
    fn bridge_transaction_manager_runtime_pack_prepare_consumes_declared_and_cached_gas() {
        let mut runtime =
            build_transaction_state_for_test(0, TransactionQueueConfig { max_size: 8 });
        let signing_key = SigningKey::from_slice(&[0x43u8; 32]).unwrap();
        let sender = address_from_signing_key(&signing_key);
        let first_rlp = signed_legacy_transaction_rlp(&signing_key, 1, 2999);
        let first_envelope = LegacyTransactionEnvelope::decode(&first_rlp).unwrap();
        let second_rlp = signed_legacy_transaction_rlp(&signing_key, 2, 2999);
        let second_envelope = LegacyTransactionEnvelope::decode(&second_rlp).unwrap();
        for (envelope, tx_rlp) in [
            (first_envelope.clone(), first_rlp),
            (second_envelope.clone(), second_rlp),
        ] {
            runtime
                .transaction_manager_runtime_queue_insert(TransactionQueueInsertInput {
                    hash: envelope.hash.0,
                    sender: sender.0,
                    nonce: envelope.nonce.to_big_endian(),
                    gas_price: envelope.gas_price.to_big_endian(),
                    gas: envelope.gas,
                    data_size: envelope.data.len(),
                    tx_rlp,
                    proposable: true,
                    last_block_number: 0,
                })
                .expect("proposable insert should succeed");
        }
        runtime
            .transaction_manager_runtime_store_gas_estimation(
                TransactionManagerGasEstimationResult {
                    hash: second_envelope.hash.0,
                    proposal_period: 7,
                    gas_used: 30_000,
                    result_rlp: vec![0xc0],
                },
            )
            .expect("cache store should succeed");

        let plan = runtime
            .transaction_manager_runtime_pack_prepare_sharded(
                63_000, 21_000, 7, 25_000, 10, 1, 0, 1,
            )
            .expect("pack prepare should consume declared/cache paths");
        assert!(plan.request_estimates.is_empty());
        assert_eq!(plan.selected_transactions.len(), 2);
        assert_eq!(plan.selected_transactions[0].hash, first_envelope.hash.0);
        assert_eq!(plan.selected_transactions[0].gas_used, 21_000);
        assert_eq!(plan.selected_transactions[1].hash, second_envelope.hash.0);
        assert_eq!(plan.selected_transactions[1].gas_used, 21_000);
        assert!(!runtime.transaction_manager_runtime_pack_abort());

        let mut cached_runtime =
            build_transaction_state_for_test(0, TransactionQueueConfig { max_size: 100 });
        cached_runtime
            .transaction_manager_runtime_queue_insert(TransactionQueueInsertInput {
                hash: second_envelope.hash.0,
                sender: sender.0,
                nonce: second_envelope.nonce.to_big_endian(),
                gas_price: second_envelope.gas_price.to_big_endian(),
                gas: second_envelope.gas,
                data_size: second_envelope.data.len(),
                tx_rlp: signed_legacy_transaction_rlp(&signing_key, 2, 2999),
                proposable: true,
                last_block_number: 0,
            })
            .expect("cached transaction insert should succeed");
        cached_runtime
            .transaction_manager_runtime_store_gas_estimation(
                TransactionManagerGasEstimationResult {
                    hash: second_envelope.hash.0,
                    proposal_period: 7,
                    gas_used: 30_000,
                    result_rlp: vec![0xc0],
                },
            )
            .expect("cache store should succeed");
        let cached_plan = cached_runtime
            .transaction_manager_runtime_pack_prepare_sharded(63_000, 21_000, 7, 0, 10, 1, 0, 1)
            .expect("cached pack prepare should finish without C++ estimate");
        assert!(cached_plan.request_estimates.is_empty());
        assert_eq!(cached_plan.selected_transactions.len(), 1);
        assert_eq!(
            cached_plan.selected_transactions[0].hash,
            second_envelope.hash.0
        );
        assert_eq!(cached_plan.selected_transactions[0].gas_used, 30_000);
        assert!(!cached_runtime.transaction_manager_runtime_pack_abort());
    }

    #[test]
    fn bridge_transaction_manager_runtime_pack_acknowledges_applied_prepare_demotions() {
        let mut runtime =
            build_transaction_state_for_test(0, TransactionQueueConfig { max_size: 100 });
        let signing_key = SigningKey::from_slice(&[0x53u8; 32]).unwrap();
        let sender = address_from_signing_key(&signing_key);
        let first_rlp = signed_legacy_transaction_rlp(&signing_key, 1, 2999);
        let first = LegacyTransactionEnvelope::decode(&first_rlp).unwrap();
        let second_rlp = signed_legacy_transaction_rlp(&signing_key, 2, 2999);
        let second = LegacyTransactionEnvelope::decode(&second_rlp).unwrap();
        for (transaction, tx_rlp) in [(first.clone(), first_rlp), (second.clone(), second_rlp)] {
            runtime
                .transaction_manager_runtime_queue_insert(TransactionQueueInsertInput {
                    hash: transaction.hash.0,
                    sender: sender.0,
                    nonce: transaction.nonce.to_big_endian(),
                    gas_price: transaction.gas_price.to_big_endian(),
                    gas: transaction.gas,
                    data_size: transaction.data.len(),
                    tx_rlp,
                    proposable: true,
                    last_block_number: 0,
                })
                .unwrap();
        }
        runtime
            .transaction_manager_runtime_store_gas_estimation(
                TransactionManagerGasEstimationResult {
                    hash: first.hash.0,
                    proposal_period: 7,
                    gas_used: 20_000,
                    result_rlp: vec![0xc0],
                },
            )
            .unwrap();

        let prepared = runtime
            .transaction_manager_runtime_pack_prepare_sharded(63_000, 21_000, 7, 0, 44, 1, 0, 1)
            .unwrap();
        assert_eq!(prepared.request_estimates.len(), 1);
        assert_eq!(prepared.request_estimates[0].hash, second.hash.0);
        assert_eq!(prepared.demoted_hashes.len(), 1);
        assert_eq!(prepared.demoted_hashes[0].hash, first.hash.0);
        assert_eq!(runtime.transaction_manager_runtime_queue_size(), 1);

        let finalized = runtime
            .transaction_manager_runtime_pack_finalize_with_estimates(vec![
                TransactionPackSessionEstimateInput {
                    hash: second.hash.0,
                    gas_used: 30_000,
                    last_block_number: 44,
                    result_rlp: vec![0xc1],
                },
            ])
            .unwrap();
        assert_eq!(finalized.demoted_hashes.len(), 1);
        assert_eq!(finalized.demoted_hashes[0].hash, first.hash.0);
        assert_eq!(finalized.selected_transactions[0].hash, second.hash.0);
    }
}
