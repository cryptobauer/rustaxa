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

use crate::ffi::rustaxa_ffi::{
    DagTransactionSaveSidecarFact, FinalizedTransactionFilterPlan,
    FinalizedTransactionStatusSidecarFact, TransactionManagerAdmissionCommandReport,
    TransactionManagerAdmissionResult, TransactionManagerAdmissionShellIntent,
    TransactionManagerDagSaveCommandReport, TransactionManagerFilterAction,
    TransactionManagerFinalChainAdmissionFact, TransactionManagerFinalizedStatusCommandReport,
    TransactionManagerGasEstimationFact, TransactionManagerGasEstimationPlan,
    TransactionManagerGasEstimationResult, TransactionManagerHashCommand,
    TransactionManagerInsertTransactionFact, TransactionManagerInsertTransactionOutcome,
    TransactionManagerPublicAdmissionCommandReport, TransactionManagerPublicInsertResult,
    TransactionManagerRuntimeAdmissionOutcome, TransactionManagerRuntimeQueueCleanupPlan,
    TransactionManagerSidecarInsertInput, TransactionManagerSidecarLookupRequest,
    TransactionManagerTransactionView, TransactionManagerTransactionViewPlan,
    TransactionManagerTransactionViewRequest, TransactionManagerValidatedInsertRuntimeFact,
    TransactionManagerVerifyNotFinalizedOutcome, TransactionManagerVerifyNotFinalizedSidecarFact,
    TransactionManagerVerifyTransactionFact, TransactionManagerVerifyTransactionOutcome,
    TransactionPackEstimateOutcome, TransactionPackSelectedTransaction,
    TransactionPackSessionCandidate, TransactionPackSessionEstimateInput,
    TransactionPackSessionStep, TransactionQueueConfig, TransactionQueueHash,
    TransactionQueueInsertInput, TransactionQueueInsertOutcome, TransactionQueuePurgePlan,
    TransactionQueueStoredTransaction, TransactionQueueTransactionGroup,
};
use crate::ffi::{
    BridgeFinalChain, BridgeStorage, BridgeTransactionManagerRuntime,
    TransactionManagerRuntimePackSession,
};
use crate::transaction::legacy_transaction_inspection_from_bytes;
use anyhow::{anyhow, ensure, Context, Result};
use ethereum_types::{H160, H256, U256};
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
    TransactionManagerSidecar,
    TransactionManagerSidecarRecoveryEntry as ConsensusTransactionManagerSidecarRecoveryEntry,
    TransactionManagerValidatedInsertFact as ConsensusTransactionManagerValidatedInsertFact,
    TransactionManagerVerifyTransactionFact as ConsensusTransactionManagerVerifyTransactionFact,
    TransactionManagerVerifyTransactionStatus, TransactionPackCandidate, TransactionPackEstimate,
    TransactionPackingPlanner,
};
use rustaxa_consensus::transaction_queue::{
    TransactionQueue, TransactionQueueAccountNonceFact, TransactionQueueDemoteStatus,
    TransactionQueueEntry, TransactionQueueInsertStatus, TransactionQueuePurgeOutcome,
};
use rustaxa_consensus::transaction_storage::{
    load_non_finalized_recovery_entries, load_stored_transactions,
    remove_non_finalized_transactions, save_non_finalized_transactions, save_transaction_count,
    transaction_finalized, NonFinalizedTransactionRecoveryEntry,
    NonFinalizedTransactionStoragePayload, StoredTransactionLookupRequest,
    STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR, STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM,
    STORED_TRANSACTION_SOURCE_MISSING, STORED_TRANSACTION_SOURCE_PENDING,
};
use rustaxa_storage::Storage;
use rustaxa_types::LegacyTransactionEnvelope;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct TransactionManagerStoredTransactionRequest {
    input_index: u64,
    hash: [u8; 32],
}

#[derive(Debug)]
struct TransactionManagerStoredTransactionLookup {
    hash: [u8; 32],
    found: bool,
    source: u8,
    old_finalized: bool,
    tx_rlp: Vec<u8>,
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

struct DagTransactionSaveOutcome {
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

const TM_TRANSACTION_VIEW_SOURCE_MISSING: u8 = 0;
const TM_TRANSACTION_VIEW_SOURCE_QUEUE: u8 = 1;
const TM_TRANSACTION_VIEW_SOURCE_NON_FINALIZED_SIDECAR: u8 = 2;
const TM_TRANSACTION_VIEW_SOURCE_RECENTLY_FINALIZED_SIDECAR: u8 = 3;
const TM_TRANSACTION_VIEW_SOURCE_STORAGE_PENDING: u8 = 4;
const TM_TRANSACTION_VIEW_SOURCE_STORAGE_FINALIZED_REGULAR: u8 = 5;
const TM_TRANSACTION_VIEW_SOURCE_STORAGE_FINALIZED_SYSTEM: u8 = 6;
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
const TRANSACTION_QUEUE_DROP_WINDOW: Duration = Duration::from_secs(600);

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

fn dag_save_command_report(
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

fn runtime_pack_next_estimable_entry(
    session: &mut TransactionManagerRuntimePackSession,
) -> Result<Option<TransactionQueueEntry>> {
    if session.stopped {
        return Ok(None);
    }

    while session.next_index < session.candidates.len() {
        let candidate = session.candidates[session.next_index].clone();
        session.next_index += 1;
        if !runtime_pack_candidate_matches_shard(&candidate, session)? {
            continue;
        }
        let decision = session
            .planner
            .consider_candidate(TransactionPackCandidate {
                hash: candidate.hash,
                declared_gas: candidate.gas,
            })?;
        if decision.should_estimate {
            session.current = Some(candidate.clone());
            return Ok(Some(candidate));
        }
    }

    Ok(None)
}

fn runtime_pack_candidate_matches_shard(
    candidate: &TransactionQueueEntry,
    session: &TransactionManagerRuntimePackSession,
) -> Result<bool> {
    if session.total_shards <= 1 {
        return Ok(true);
    }
    ensure!(
        session.node_shard < session.total_shards,
        "TM_RUNTIME_PACK_SHARD_OUT_OF_RANGE"
    );
    ensure!(
        session.shard_period_interval != 0,
        "TM_RUNTIME_PACK_SHARD_INTERVAL_ZERO"
    );

    let sender_prefix = legacy_transaction_shard_sender_prefix(candidate.sender);
    let shard = sender_prefix.wrapping_add(session.proposal_period / session.shard_period_interval)
        % u64::from(session.total_shards);
    Ok(shard == u64::from(session.node_shard))
}

/// Returns the legacy DAG transaction-sharding sender prefix.
///
/// C++ `DagBlockProposer::getShardedTrxs` parses
/// `sender.toString().substr(0, 10)` as hexadecimal, which is the first five
/// address bytes. The Rust runtime keeps that exact 40-bit prefix so
/// multi-shard DAG proposal selects the same transactions as the compatibility
/// proposer.
fn legacy_transaction_shard_sender_prefix(sender: H160) -> u64 {
    u64::from_be_bytes([
        0,
        0,
        0,
        sender.0[0],
        sender.0[1],
        sender.0[2],
        sender.0[3],
        sender.0[4],
    ])
}

fn runtime_pack_session_final_step(
    session: &TransactionManagerRuntimePackSession,
) -> TransactionPackSessionStep {
    TransactionPackSessionStep {
        request_estimate: false,
        candidate: transaction_pack_session_empty_candidate(),
        selected_transactions: session
            .selected
            .iter()
            .map(|(entry, gas_used)| TransactionPackSelectedTransaction {
                hash: entry.hash.0,
                gas_used: *gas_used,
                tx_rlp: entry.rlp.clone(),
            })
            .collect(),
        demoted_hashes: runtime_hashes_to_bridge(session.demoted_hashes.clone()),
        stopped: session.stopped,
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
) -> TransactionQueuePurgePlan {
    TransactionQueuePurgePlan {
        removed_count: outcome.removed_hashes.len(),
        removed_hashes: runtime_hashes_to_bridge(outcome.removed_hashes),
    }
}

fn runtime_queue_account_nonce_facts_from_final_chain(
    final_chain: &BridgeFinalChain,
    proposable_accounts: Vec<H160>,
) -> Result<Vec<TransactionQueueAccountNonceFact>> {
    proposable_accounts
        .into_iter()
        .map(|sender| {
            let lookup = final_chain
                .get_account(&sender.0)
                .context("TM_RUNTIME_QUEUE_PURGE_ACCOUNT_LOOKUP_FAILED")?;
            Ok(TransactionQueueAccountNonceFact {
                sender,
                account_found: lookup.found,
                account_nonce: U256::from(lookup.nonce),
            })
        })
        .collect()
}

fn transaction_manager_runtime_storage(
    runtime: &BridgeTransactionManagerRuntime,
) -> Result<&Storage> {
    runtime
        .storage
        .as_deref()
        .context("TM_RUNTIME_STORAGE_UNAVAILABLE")
}

/// Plans and persists accepted DAG-block transactions through the Rust manager runtime.
fn save_transactions_from_dag_block_with_runtime(
    runtime: &mut BridgeTransactionManagerRuntime,
    facts: Vec<DagTransactionSaveSidecarFact>,
) -> Result<DagTransactionSaveOutcome> {
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

    if !accepted_payloads.is_empty() {
        save_non_finalized_transactions(storage, accepted_payloads, plan.target_transaction_count)?;
    }

    for (accepted_entry, payload) in accepted.iter_mut().zip(plan.accepted_transactions) {
        accepted_entry.erased_from_queue = runtime.queue.erase(payload.hash);
        runtime
            .sidecar
            .insert_non_finalized(payload.hash, payload.trx_rlp)
            .context("TM_RUNTIME_DAG_TX_INSERT")?;
    }
    runtime
        .sidecar
        .set_transaction_count(plan.target_transaction_count);

    Ok(DagTransactionSaveOutcome { accepted })
}

/// Applies DAG transaction persistence and returns a typed command report.
pub fn save_transactions_from_dag_block_command_report_with_runtime(
    runtime: &mut BridgeTransactionManagerRuntime,
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
    runtime: &mut BridgeTransactionManagerRuntime,
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
pub fn update_finalized_transactions_status_command_report_with_runtime(
    runtime: &mut BridgeTransactionManagerRuntime,
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
pub fn update_finalized_transactions_status_command_report_with_runtime_and_final_chain(
    runtime: &mut BridgeTransactionManagerRuntime,
    final_chain: &BridgeFinalChain,
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
    let mut report = finalized_status_command_report(&outcome);
    if report.purge_transaction_queue {
        let cleanup = runtime.transaction_manager_runtime_queue_cleanup_with_final_chain(
            final_chain,
            false,
            0,
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
    runtime: &BridgeTransactionManagerRuntime,
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

const TM_STORED_TX_SOURCE_MISSING: u8 = STORED_TRANSACTION_SOURCE_MISSING;
const TM_STORED_TX_SOURCE_PENDING: u8 = STORED_TRANSACTION_SOURCE_PENDING;
const TM_STORED_TX_SOURCE_FINALIZED_REGULAR: u8 = STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR;
const TM_STORED_TX_SOURCE_FINALIZED_SYSTEM: u8 = STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM;
const TM_VERIFY_NOT_FINALIZED_SOURCE_NONE: u8 = 0;
const TM_VERIFY_NOT_FINALIZED_SOURCE_RECENT_SIDECAR: u8 = 1;
const TM_VERIFY_NOT_FINALIZED_SOURCE_STORAGE: u8 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
struct StoredTransactionIdentity {
    sender: [u8; 20],
    nonce: U256,
}

fn transaction_manager_transaction_view_source_from_stored_transaction_source(
    source: u8,
) -> Result<u8> {
    match source {
        TM_STORED_TX_SOURCE_MISSING => Ok(TM_TRANSACTION_VIEW_SOURCE_MISSING),
        TM_STORED_TX_SOURCE_PENDING => Ok(TM_TRANSACTION_VIEW_SOURCE_STORAGE_PENDING),
        TM_STORED_TX_SOURCE_FINALIZED_REGULAR => {
            Ok(TM_TRANSACTION_VIEW_SOURCE_STORAGE_FINALIZED_REGULAR)
        }
        TM_STORED_TX_SOURCE_FINALIZED_SYSTEM => {
            Ok(TM_TRANSACTION_VIEW_SOURCE_STORAGE_FINALIZED_SYSTEM)
        }
        _ => Err(anyhow!("TM_TRANSACTION_VIEW_UNKNOWN_STORED_SOURCE")),
    }
}

fn transaction_manager_transaction_view_source_from_sidecar_transaction_source(
    source: u8,
) -> Result<u8> {
    match source {
        1 => Ok(TM_TRANSACTION_VIEW_SOURCE_NON_FINALIZED_SIDECAR),
        2 => Ok(TM_TRANSACTION_VIEW_SOURCE_RECENTLY_FINALIZED_SIDECAR),
        _ => Err(anyhow!("TM_TRANSACTION_VIEW_UNKNOWN_SIDECAR_SOURCE")),
    }
}

fn bounded_transaction_view_count(requests_len: usize, max_count: u64) -> usize {
    match max_count {
        0 => requests_len,
        _ => (max_count.min(requests_len as u64)) as usize,
    }
}

fn transaction_manager_load_stored_transactions_from_storage(
    storage: &Storage,
    requests: Vec<TransactionManagerStoredTransactionRequest>,
) -> Result<Vec<TransactionManagerStoredTransactionLookup>> {
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
            Ok(TransactionManagerStoredTransactionLookup {
                hash: lookup.hash.0,
                found: lookup.found,
                source: lookup.source,
                old_finalized: false,
                tx_rlp: lookup.tx_rlp,
            })
        })
        .collect()
}

fn transaction_manager_load_proposal_transactions_with_final_chain_from_storage(
    storage: &Storage,
    final_chain: &BridgeFinalChain,
    proposal_period: u64,
    requests: Vec<TransactionManagerStoredTransactionRequest>,
) -> Result<Vec<TransactionManagerStoredTransactionLookup>> {
    let lookups = transaction_manager_load_stored_transactions_from_storage(storage, requests)?;
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
            let account = final_chain
                .get_account_at_block(proposal_period, &identity.sender)
                .context("TM_PROPOSAL_FINAL_CHAIN_ACCOUNT_LOOKUP_FAILED")?;
            if account.found && U256::from(account.nonce) > identity.nonce {
                lookup.found = false;
                lookup.old_finalized = true;
                lookup.tx_rlp.clear();
            }
            Ok(lookup)
        })
        .collect()
}

fn is_finalized_stored_transaction_source(source: u8) -> bool {
    source == TM_STORED_TX_SOURCE_FINALIZED_REGULAR
        || source == TM_STORED_TX_SOURCE_FINALIZED_SYSTEM
}

fn inspect_stored_transaction_identity(
    tx_rlp: &[u8],
    source: u8,
) -> Result<StoredTransactionIdentity> {
    let tx = if source == TM_STORED_TX_SOURCE_FINALIZED_SYSTEM {
        LegacyTransactionEnvelope::decode_system(tx_rlp)
    } else {
        LegacyTransactionEnvelope::decode(tx_rlp)
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

fn transaction_manager_runtime_lookup_transaction_views_inner(
    runtime: &BridgeTransactionManagerRuntime,
    requests: Vec<TransactionManagerTransactionViewRequest>,
    max_count: u64,
    mut transaction_lookup: impl FnMut(
        Vec<TransactionManagerStoredTransactionRequest>,
    ) -> Result<Vec<TransactionManagerStoredTransactionLookup>>,
) -> Result<TransactionManagerTransactionViewPlan> {
    let total_requests = requests.len();
    let requested_count = bounded_transaction_view_count(total_requests, max_count) as u64;
    let mut views = Vec::with_capacity(requested_count as usize);
    let mut sidecar_requests = Vec::new();
    let mut sidecar_view_indexes = Vec::new();

    for request in requests.into_iter().take(requested_count as usize) {
        let hash = H256::from(request.hash);
        let queue_view = runtime.queue.transaction(hash);

        let mut view = TransactionManagerTransactionView {
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
                    transaction_manager_transaction_view_source_from_sidecar_transaction_source(
                        lookup.source,
                    )
                    .context("TM_RUNTIME_TRANSACTION_VIEW_SIDECAR_SOURCE")?;
                views[view_index].found = true;
                views[view_index].source = source;
                views[view_index].tx_rlp = lookup.trx_rlp;
            } else {
                storage_requests.push(TransactionManagerStoredTransactionRequest {
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
                    transaction_manager_transaction_view_source_from_stored_transaction_source(
                        lookup.source,
                    )
                    .context("TM_RUNTIME_TRANSACTION_VIEW_STORED_SOURCE")?;
            }
        }
    }

    Ok(TransactionManagerTransactionViewPlan {
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

/// Verifies transaction hashes against Rust runtime sidecars with sender nonce
/// facts supplied by the external-EVM compatibility boundary.
pub fn transaction_manager_verify_not_finalized_with_runtime(
    runtime: &BridgeTransactionManagerRuntime,
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
    runtime: &mut BridgeTransactionManagerRuntime,
) -> Result<()> {
    let entries = transaction_manager_load_nonfinalized_recovery_inputs_from_storage(
        transaction_manager_runtime_storage(runtime)?,
    )?;
    runtime.insert_recovery_entries(entries).map(|_| ())
}

/// Creates the Rust-owned TransactionManager runtime for Rust-enabled manager shims.
///
/// The runtime owns both the live manager sidecars and the transaction queue
/// metadata/payload state. C++ supplies materialized transaction facts at method
/// boundaries and remains responsible for events, logging, historical account
/// reads, and gas estimation. Latest-state admission, DAG-save, verification,
/// and finalized-account queue purge can source account facts directly from
/// Rust FinalChain through runtime APIs.
pub fn create_transaction_manager_runtime(
    initial_transaction_count: u64,
    config: TransactionQueueConfig,
) -> Box<BridgeTransactionManagerRuntime> {
    create_transaction_manager_runtime_inner(initial_transaction_count, config, None)
}

/// Creates the production Rust-owned TransactionManager runtime with durable storage attached.
///
/// The runtime clones the underlying `Arc<rustaxa_storage::Storage>` from the
/// generic constructor facade and becomes the storage authority for migrated
/// transaction-manager persistence, recovery, lookup, and finalized-status
/// routes. C++ should keep only this runtime handle after construction.
pub fn create_transaction_manager_runtime_from_storage(
    storage: &BridgeStorage,
    initial_transaction_count: u64,
    config: TransactionQueueConfig,
) -> Box<BridgeTransactionManagerRuntime> {
    create_transaction_manager_runtime_inner(
        initial_transaction_count,
        config,
        Some(storage.0.clone()),
    )
}

fn create_transaction_manager_runtime_inner(
    initial_transaction_count: u64,
    config: TransactionQueueConfig,
    storage: Option<std::sync::Arc<Storage>>,
) -> Box<BridgeTransactionManagerRuntime> {
    let gas_estimation_cache_size = config.max_size / 10;
    let gas_estimation_cache_delete_step = config.max_size / 100;
    Box::new(BridgeTransactionManagerRuntime {
        sidecar: TransactionManagerSidecar::new_with_gas_estimation_cache(
            initial_transaction_count,
            gas_estimation_cache_size,
            gas_estimation_cache_delete_step,
        ),
        queue: TransactionQueue::new(config.max_size as u64),
        storage,
        last_drop_observed: None,
        transaction_pack_session: None,
    })
}

impl BridgeTransactionManagerRuntime {
    /// Begins one runtime-owned transaction packing session for a DAG proposer shard.
    ///
    /// Sharding is applied before gas estimation using the legacy sender-prefix
    /// rule, so C++ only materializes and estimates candidates that Rust may
    /// actually select for the configured proposer shard.
    #[allow(clippy::too_many_arguments)]
    pub fn transaction_manager_runtime_pack_begin_sharded(
        &mut self,
        weight_limit: u64,
        min_transaction_gas: u64,
        proposal_period: u64,
        estimate_gas_limit: u64,
        last_block_number: u64,
        total_shards: u16,
        node_shard: u16,
        shard_period_interval: u64,
    ) -> Result<()> {
        ensure!(
            self.transaction_pack_session.is_none(),
            "TM_RUNTIME_PACK_SESSION_ALREADY_ACTIVE"
        );
        ensure!(total_shards != 0, "TM_RUNTIME_PACK_TOTAL_SHARDS_ZERO");
        ensure!(
            node_shard < total_shards,
            "TM_RUNTIME_PACK_NODE_SHARD_OUT_OF_RANGE"
        );
        ensure!(
            total_shards <= 1 || shard_period_interval != 0,
            "TM_RUNTIME_PACK_SHARD_INTERVAL_ZERO"
        );
        let planner = TransactionPackingPlanner::new(weight_limit, min_transaction_gas)?;
        let candidates = self
            .queue
            .ordered_transactions(planner.max_candidate_count());
        self.transaction_pack_session = Some(TransactionManagerRuntimePackSession {
            planner,
            proposal_period,
            estimate_gas_limit,
            last_block_number,
            total_shards,
            node_shard,
            shard_period_interval,
            candidates,
            next_index: 0,
            current: None,
            selected: Vec::new(),
            demoted_hashes: Vec::new(),
            stopped: false,
        });
        Ok(())
    }

    /// Requests the next packed transaction candidate or final session outcome.
    pub fn transaction_manager_runtime_pack_request_next(
        &mut self,
    ) -> Result<TransactionPackSessionStep> {
        let mut session = self
            .transaction_pack_session
            .take()
            .context("TM_RUNTIME_PACK_SESSION_NOT_ACTIVE")?;
        let step = match self.transaction_manager_runtime_pack_step_inner(&mut session) {
            Ok(step) => step,
            Err(err) => {
                self.transaction_pack_session = Some(session);
                return Err(err).context("TM_RUNTIME_PACK_REQUEST_NEXT");
            }
        };
        if step.request_estimate {
            self.transaction_pack_session = Some(session);
        }
        Ok(step)
    }

    /// Records one C++ gas estimate and returns the next Rust-driven request or final output.
    pub fn transaction_manager_runtime_pack_record_estimate_step(
        &mut self,
        input: TransactionPackSessionEstimateInput,
    ) -> Result<TransactionPackSessionStep> {
        let mut session = self
            .transaction_pack_session
            .take()
            .context("TM_RUNTIME_PACK_SESSION_NOT_ACTIVE")?;

        if let Err(err) =
            self.transaction_manager_runtime_pack_record_estimate_inner(&mut session, input)
        {
            self.transaction_pack_session = Some(session);
            return Err(err).context("TM_RUNTIME_PACK_RECORD_ESTIMATE");
        }

        let out = match self.transaction_manager_runtime_pack_step_inner(&mut session) {
            Ok(out) => out,
            Err(err) => {
                self.transaction_pack_session = Some(session);
                return Err(err).context("TM_RUNTIME_PACK_RECORD_ESTIMATE");
            }
        };

        if out.request_estimate {
            self.transaction_pack_session = Some(session);
        }
        Ok(out)
    }

    fn transaction_manager_runtime_pack_step_inner(
        &mut self,
        session: &mut TransactionManagerRuntimePackSession,
    ) -> Result<TransactionPackSessionStep> {
        loop {
            let Some(candidate) = runtime_pack_next_estimable_entry(session)? else {
                return Ok(runtime_pack_session_final_step(session));
            };

            if candidate.gas <= session.estimate_gas_limit {
                let input = TransactionPackSessionEstimateInput {
                    hash: candidate.hash.0,
                    gas_used: candidate.gas,
                    last_block_number: session.last_block_number,
                    result_rlp: Vec::new(),
                };
                self.transaction_manager_runtime_pack_record_estimate_inner(session, input)?;
                continue;
            }

            if let Some(cached) = self
                .sidecar
                .gas_estimation_cache_get(candidate.hash, session.proposal_period)
                .context("TM_RUNTIME_PACK_GAS_ESTIMATION_CACHE_GET")?
            {
                let input = TransactionPackSessionEstimateInput {
                    hash: candidate.hash.0,
                    gas_used: cached.gas_used,
                    last_block_number: session.last_block_number,
                    result_rlp: Vec::new(),
                };
                self.transaction_manager_runtime_pack_record_estimate_inner(session, input)?;
                continue;
            }

            return Ok(TransactionPackSessionStep {
                request_estimate: true,
                candidate: transaction_pack_candidate_from_entry(Some(candidate))?,
                selected_transactions: Vec::new(),
                demoted_hashes: Vec::new(),
                stopped: session.stopped,
            });
        }
    }

    fn transaction_manager_runtime_pack_record_estimate_inner(
        &mut self,
        session: &mut TransactionManagerRuntimePackSession,
        input: TransactionPackSessionEstimateInput,
    ) -> Result<TransactionPackEstimateOutcome> {
        let Some(candidate) = session.current.take() else {
            return Err(anyhow!("TM_RUNTIME_PACK_NO_ACTIVE_CANDIDATE"));
        };
        let estimate_hash = H256::from(input.hash);
        if candidate.hash != estimate_hash {
            session.current = Some(candidate);
            return Err(anyhow!("TM_RUNTIME_PACK_HASH_MISMATCH"));
        }

        let outcome = session.planner.record_estimate(TransactionPackEstimate {
            hash: H256::from(input.hash),
            gas_used: input.gas_used,
        })?;

        if outcome.demote_to_non_proposable {
            let demote_outcome = self.queue.demote(estimate_hash, input.last_block_number);
            if matches!(demote_outcome.status, TransactionQueueDemoteStatus::Demoted) {
                session.demoted_hashes.push(estimate_hash);
            }
        }
        if !input.result_rlp.is_empty() {
            self.sidecar
                .gas_estimation_cache_insert(
                    estimate_hash,
                    session.proposal_period,
                    input.gas_used,
                    input.result_rlp,
                )
                .context("TM_RUNTIME_PACK_GAS_ESTIMATION_CACHE_STORE")?;
        }
        if outcome.selected {
            session.selected.push((candidate, outcome.gas_used));
        }
        if outcome.stop {
            session.stopped = true;
        }
        Ok(TransactionPackEstimateOutcome {
            hash: outcome.hash.0,
            selected: outcome.selected,
            demote_to_non_proposable: outcome.demote_to_non_proposable,
            stop: outcome.stop,
            gas_used: outcome.gas_used,
        })
    }

    /// Aborts and clears the active runtime packing session.
    pub fn transaction_manager_runtime_pack_abort(&mut self) -> bool {
        self.transaction_pack_session.take().is_some()
    }

    /// Plans one public gas-estimation request using Rust-owned cache policy.
    ///
    /// Rust decides the declared-gas fast path and cache hits. C++ must call the
    /// EVM only when `requires_evm_call` is true, then feed the opaque
    /// `ExecutionResult` RLP back through `transaction_manager_runtime_store_gas_estimation`.
    pub fn transaction_manager_runtime_plan_gas_estimation(
        &self,
        fact: TransactionManagerGasEstimationFact,
    ) -> Result<TransactionManagerGasEstimationPlan> {
        let hash = H256::from(fact.hash);
        ensure!(!hash.is_zero(), "TM_RUNTIME_GAS_ESTIMATION_HASH_ZERO");

        if fact.declared_gas <= fact.estimate_gas_limit {
            return Ok(TransactionManagerGasEstimationPlan {
                use_declared_gas: true,
                cache_hit: false,
                requires_evm_call: false,
                gas_used: fact.declared_gas,
                result_rlp: Vec::new(),
            });
        }

        if let Some(cached) = self
            .sidecar
            .gas_estimation_cache_get(hash, fact.proposal_period)
            .context("TM_RUNTIME_GAS_ESTIMATION_CACHE_GET")?
        {
            return Ok(TransactionManagerGasEstimationPlan {
                use_declared_gas: false,
                cache_hit: true,
                requires_evm_call: false,
                gas_used: cached.gas_used,
                result_rlp: cached.result_rlp,
            });
        }

        Ok(TransactionManagerGasEstimationPlan {
            use_declared_gas: false,
            cache_hit: false,
            requires_evm_call: true,
            gas_used: 0,
            result_rlp: Vec::new(),
        })
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

    /// Returns the authoritative Rust-mode manager transaction count.
    pub fn transaction_manager_runtime_transaction_count(&self) -> u64 {
        self.sidecar.transaction_count()
    }

    /// Returns queue-only payload views.
    pub fn transaction_manager_runtime_queue_lookup_transaction_views(
        &self,
        requests: Vec<TransactionManagerTransactionViewRequest>,
    ) -> Result<Vec<TransactionManagerTransactionView>> {
        Ok(requests
            .into_iter()
            .map(|request| {
                let entry = self.queue.transaction(H256::from(request.hash));
                match entry {
                    Some(entry) => TransactionManagerTransactionView {
                        input_index: request.input_index,
                        hash: request.hash,
                        found: true,
                        source: TM_TRANSACTION_VIEW_SOURCE_QUEUE,
                        old_finalized: false,
                        tx_rlp: entry.rlp,
                    },
                    None => TransactionManagerTransactionView {
                        input_index: request.input_index,
                        hash: request.hash,
                        found: false,
                        source: TM_TRANSACTION_VIEW_SOURCE_MISSING,
                        old_finalized: false,
                        tx_rlp: Vec::new(),
                    },
                }
            })
            .collect())
    }

    /// Returns Rust's known-transaction decision for a canonical hash.
    ///
    /// Production C++ shims call this hash-only API so queue membership and
    /// sidecar membership are derived exclusively from the Rust runtime. The
    /// older fact-shaped helper remains only as a compatibility wrapper for
    /// lower-level sidecar tests and non-production callers.
    pub fn transaction_manager_runtime_is_transaction_known_hash(
        &self,
        hash: &[u8; 32],
    ) -> Result<bool> {
        self.sidecar
            .is_transaction_known(TransactionManagerKnownFact {
                hash: H256::from(*hash),
                queue_known: self.queue.is_transaction_known(H256::from(*hash)),
            })
            .context("TM_RUNTIME_IS_TRANSACTION_KNOWN")
    }

    /// Returns bounded, source-ordered runtime payload views from queue, sidecars, and storage.
    pub fn transaction_manager_runtime_lookup_transaction_views(
        &self,
        requests: Vec<TransactionManagerTransactionViewRequest>,
        max_count: u64,
    ) -> Result<TransactionManagerTransactionViewPlan> {
        let storage = transaction_manager_runtime_storage(self)?;
        transaction_manager_runtime_lookup_transaction_views_inner(
            self,
            requests,
            max_count,
            |stored_requests| {
                transaction_manager_load_stored_transactions_from_storage(storage, stored_requests)
            },
        )
    }

    /// Returns bounded, source-ordered runtime payload views including proposal-period filtering.
    pub fn transaction_manager_runtime_lookup_proposal_transaction_views(
        &self,
        final_chain: &BridgeFinalChain,
        proposal_period: u64,
        requests: Vec<TransactionManagerTransactionViewRequest>,
        max_count: u64,
    ) -> Result<TransactionManagerTransactionViewPlan> {
        let storage = transaction_manager_runtime_storage(self)?;
        transaction_manager_runtime_lookup_transaction_views_inner(
            self,
            requests,
            max_count,
            |stored_requests| {
                transaction_manager_load_proposal_transactions_with_final_chain_from_storage(
                    storage,
                    final_chain,
                    proposal_period,
                    stored_requests,
                )
            },
        )
    }

    /// Returns non-finalized/recently-finalized sidecar payload views.
    pub fn transaction_manager_runtime_lookup_non_finalized_transaction_views(
        &self,
        requests: Vec<TransactionManagerTransactionViewRequest>,
    ) -> Result<Vec<TransactionManagerTransactionView>> {
        let lookups = self
            .sidecar
            .lookup_payloads_ordered(
                requests
                    .into_iter()
                    .map(|request| (request.input_index, H256::from(request.hash)))
                    .collect(),
            )
            .context("TM_RUNTIME_TRANSACTION_VIEW_NON_FINALIZED_LOOKUP")?;
        Ok(lookups
            .into_iter()
            .map(|lookup| {
                let found = lookup.found
                    && lookup.source
                        == rustaxa_consensus::transaction_manager::TransactionManagerSidecarLookup::SOURCE_NON_FINALIZED;
                TransactionManagerTransactionView {
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
                }
            })
            .collect())
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

    /// Returns current non-finalized sidecar size.
    pub fn transaction_manager_runtime_non_finalized_size(&self) -> usize {
        self.sidecar.non_finalized_size()
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
            .sidecar
            .insert_recovery_entries(entries)
            .context("TM_RUNTIME_RECOVERY_INSERT")? as u64)
    }

    /// Inserts transaction metadata and canonical bytes into the Rust-owned queue.
    pub fn transaction_manager_runtime_queue_insert(
        &mut self,
        input: TransactionQueueInsertInput,
    ) -> Result<TransactionQueueInsertOutcome> {
        let proposable = input.proposable;
        let outcome = self
            .queue
            .insert(runtime_queue_entry_from_insert_input(&input), proposable)?;
        if matches!(outcome.status, TransactionQueueInsertStatus::Overflow)
            || !outcome.overflow_removed_hashes.is_empty()
        {
            self.last_drop_observed = Some(Instant::now());
        }
        Ok(TransactionQueueInsertOutcome {
            status: queue_status_to_ffi(outcome.status),
            inserted_hash_found: outcome.inserted_hash.is_some(),
            inserted_hash: outcome.inserted_hash.unwrap_or_default().0,
            demoted_hashes: runtime_hashes_to_bridge(outcome.demoted_hashes),
            overflow_removed_hashes: runtime_hashes_to_bridge(outcome.overflow_removed_hashes),
        })
    }

    /// Returns the Rust-owned public insert precheck for known transactions.
    ///
    /// C++ calls this before signature/gas verification so known hashes keep the
    /// legacy fast path while Rust remains authoritative for queue-known plus
    /// sidecar membership.
    pub fn transaction_manager_runtime_insert_transaction_precheck(
        &self,
        hash: &[u8; 32],
    ) -> Result<TransactionManagerInsertTransactionOutcome> {
        let tx_hash = H256::from(*hash);
        let hash_known = self
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
            TransactionQueueInsertOutcome {
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
            demoted_hashes: queue_outcome.demoted_hashes,
            overflow_removed_hashes: queue_outcome.overflow_removed_hashes,
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

    /// Returns proposer transaction payloads grouped by sender and nonce order.
    pub fn transaction_manager_runtime_queue_all_transaction_groups(
        &self,
    ) -> Vec<TransactionQueueTransactionGroup> {
        self.queue
            .all_transaction_groups()
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

    /// Returns true when the queue contains a transaction hash.
    #[cfg(test)]
    fn transaction_manager_runtime_queue_contains(&self, hash: &[u8; 32]) -> bool {
        self.queue.contains(H256::from(*hash))
    }

    /// Returns the number of proposable transactions.
    pub fn transaction_manager_runtime_queue_size(&self) -> usize {
        self.queue.size() as usize
    }

    /// Applies finalized-block expiry to non-proposable queue state.
    pub fn transaction_manager_runtime_queue_block_finalized(
        &mut self,
        block_number: u64,
    ) -> Vec<TransactionQueueHash> {
        runtime_hashes_to_bridge(self.queue.block_finalized(block_number))
    }

    /// Applies Rust-owned queue cleanup by sourcing account nonce facts from Rust FinalChain.
    ///
    /// This keeps finalized-account purge fact sourcing inside Rust and mutates the
    /// Rust-owned queue directly without C++ account-lookup involvement.
    pub fn transaction_manager_runtime_queue_cleanup_with_final_chain(
        &mut self,
        final_chain: &BridgeFinalChain,
        apply_block_finalized: bool,
        block_number: u64,
    ) -> Result<TransactionManagerRuntimeQueueCleanupPlan> {
        let account_nonce_facts = runtime_queue_account_nonce_facts_from_final_chain(
            final_chain,
            self.queue.proposable_accounts(),
        )?;
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

    /// Returns true while the overflow/drop observation window remains active.
    pub fn transaction_manager_runtime_queue_transactions_dropped(&self) -> bool {
        self.last_drop_observed
            .is_some_and(|observed| observed.elapsed() < TRANSACTION_QUEUE_DROP_WINDOW)
    }

    /// Returns true when non-proposable queue state exceeds the configured limit.
    pub fn transaction_manager_runtime_queue_non_proposable_over_limit(&self) -> bool {
        self.queue.non_proposable_transactions_over_the_limit()
    }

    /// Returns the minimum big-endian gas price needed for next-block inclusion.
    pub fn transaction_manager_runtime_queue_min_gas_price_for_block_inclusion(
        &self,
        limit: u64,
    ) -> [u8; 32] {
        self.queue
            .min_gas_price_for_block_inclusion(limit)
            .to_big_endian()
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
        BridgeMetadataStorageQueries, BridgeStorage, BridgeTransactionStorageQueries,
    };
    use crate::storage::{create_metadata_storage_queries, create_transaction_storage_queries};
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_storage::StatusField;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    fn system_transaction_rlp(nonce: u64) -> Vec<u8> {
        let mut stream = RlpStream::new_list(9);
        stream.append(&U256::from(nonce));
        stream.append(&U256::zero());
        stream.append(&0u64);
        stream.append(&H160::zero());
        stream.append(&U256::zero());
        stream.append(&Vec::<u8>::new());
        stream.append(&U256::from(1u64));
        stream.append(&U256::zero());
        stream.append(&U256::zero());
        stream.out().to_vec()
    }

    #[test]
    fn stored_transaction_identity_inspects_regular_and_system_rlp() {
        let signing_key = SigningKey::from_slice(&[0x31u8; 32]).unwrap();
        let sender = address_from_signing_key(&signing_key);
        let transaction_rlp = signed_legacy_transaction_rlp(&signing_key, 7, 2999);

        let identity = inspect_stored_transaction_identity(
            &transaction_rlp,
            TM_STORED_TX_SOURCE_FINALIZED_REGULAR,
        )
        .expect("signed transaction should inspect");

        assert_eq!(identity.sender, sender.0);
        assert_eq!(identity.nonce, U256::from(7u64));

        let system_identity = inspect_stored_transaction_identity(
            &system_transaction_rlp(3),
            TM_STORED_TX_SOURCE_FINALIZED_SYSTEM,
        )
        .expect("system transaction should inspect");

        assert_eq!(system_identity.sender, rustaxa_types::TARAXA_SYSTEM_ACCOUNT);
        assert_eq!(system_identity.nonce, U256::from(3u64));
        assert!(inspect_stored_transaction_identity(
            &system_transaction_rlp(3),
            TM_STORED_TX_SOURCE_FINALIZED_REGULAR
        )
        .unwrap_err()
        .to_string()
        .contains("SENDER_RECOVERY_FAILED"));
    }

    #[test]
    fn proposal_transaction_lookup_filters_old_finalized_with_block_scoped_account() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_transaction_manager_proposal_lookup_filter");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        let signing_key = SigningKey::from_slice(&[0x32u8; 32]).unwrap();
        let sender = address_from_signing_key(&signing_key);
        let sender_bytes: [u8; 20] = sender.into();
        let transaction_rlp = signed_legacy_transaction_rlp(&signing_key, 0, 2999);
        let transaction_hash = keccak256(&transaction_rlp).0;
        let pbft_key = SigningKey::from_slice(&[0x33u8; 32]).unwrap();
        let pbft_block = signed_pbft_block(&pbft_key, 1, 100);

        storage
            .0
            .period()
            .write(
                1,
                &period_data_rlp_with_pbft(&pbft_block, std::slice::from_ref(&transaction_rlp)),
            )
            .expect("period data should persist");

        let final_chain = crate::final_chain::create_final_chain(
            &storage,
            100_000,
            0,
            vec![crate::ffi::rustaxa_ffi::GenesisAccount {
                address: sender_bytes,
                balance: U256::from(1_000_000u64).to_big_endian().to_vec(),
            }],
            vec![],
            crate::ffi::rustaxa_ffi::GenesisDposConfig {
                eligibility_balance_threshold: vec![],
                vote_eligibility_balance_step: U256::one().to_big_endian().to_vec(),
                validator_maximum_stake: U256::from(30_000u64).to_big_endian().to_vec(),
                minimum_deposit: vec![],
                commission_change_delta: 0,
                commission_change_frequency: 0,
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("final chain should initialize");

        final_chain
            .finalize_block(
                pbft_block,
                vec![crate::ffi::rustaxa_ffi::FinalizationTransaction {
                    hash: transaction_hash,
                    sender: sender_bytes,
                    receiver_found: true,
                    receiver: [0x44u8; 20],
                    nonce: 0,
                    value: U256::from(3u64).to_big_endian().to_vec(),
                    gas_price: U256::from(2u64).to_big_endian().to_vec(),
                    gas_limit: 21_000,
                    data: vec![],
                    rlp: transaction_rlp.clone(),
                }],
                vec![],
            )
            .expect("finalization should create block-scoped account snapshot");

        let before = transaction_manager_load_proposal_transactions_with_final_chain_from_storage(
            &storage.0,
            &final_chain,
            0,
            vec![TransactionManagerStoredTransactionRequest {
                input_index: 4,
                hash: transaction_hash,
            }],
        )
        .expect("proposal lookup at genesis should keep transaction");
        assert!(before[0].found);
        assert!(!before[0].old_finalized);
        assert_eq!(before[0].tx_rlp, transaction_rlp);

        let after = transaction_manager_load_proposal_transactions_with_final_chain_from_storage(
            &storage.0,
            &final_chain,
            1,
            vec![TransactionManagerStoredTransactionRequest {
                input_index: 4,
                hash: transaction_hash,
            }],
        )
        .expect("proposal lookup at finalized period should filter old transaction");
        assert!(!after[0].found);
        assert!(after[0].old_finalized);
        assert!(after[0].tx_rlp.is_empty());

        let _ = fs::remove_dir_all(temp_dir);
    }

    fn transaction_manager_view_request(
        input_index: u64,
        hash: u8,
    ) -> TransactionManagerTransactionViewRequest {
        TransactionManagerTransactionViewRequest {
            input_index,
            hash: [hash; 32],
        }
    }

    #[test]
    fn bridge_transaction_manager_runtime_lookup_transaction_views_enforces_source_precedence_and_bounds(
    ) {
        let temp_dir =
            unique_temp_dir("rustaxa_bridge_transaction_manager_runtime_lookup_transaction_views");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        let mut runtime = create_transaction_manager_runtime_from_storage(
            &storage,
            11,
            TransactionQueueConfig { max_size: 32 },
        );
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input_for_sender(
                1, [9u8; 20], 7, true,
            ))
            .expect("queue insert should seed");

        runtime
            .transaction_manager_runtime_insert_non_finalized(
                TransactionManagerSidecarInsertInput {
                    hash: [2u8; 32],
                    trx_rlp: vec![0x22],
                },
            )
            .expect("non-finalized sidecar insert should seed");
        runtime
            .transaction_manager_runtime_initialize_recently_finalized_payloads(
                10,
                vec![TransactionManagerSidecarInsertInput {
                    hash: [3u8; 32],
                    trx_rlp: vec![0x33],
                }],
            )
            .expect("recently-finalized payload initialization should move source");

        storage
            .0
            .transaction()
            .write(H256::from([4u8; 32]), &[0x44])
            .expect("storage pending payload should persist");

        storage
            .0
            .transaction()
            .write_location(H256::from([5u8; 32]), 99, 0, false)
            .expect("finalized location should persist");
        let mut txs = RlpStream::new_list(1);
        txs.append_raw(&[0x55], 1);
        let mut period_data = RlpStream::new_list(5);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&txs.out(), 1);
        period_data.append_raw(&[0xC0], 1);
        storage
            .0
            .period()
            .write(99, &period_data.out().as_ref().to_vec())
            .expect("finalized period data should persist");

        let plan = runtime
            .transaction_manager_runtime_lookup_transaction_views(
                vec![
                    transaction_manager_view_request(1, 1),
                    transaction_manager_view_request(2, 2),
                    transaction_manager_view_request(3, 3),
                    transaction_manager_view_request(4, 4),
                    transaction_manager_view_request(5, 5),
                    transaction_manager_view_request(6, 6),
                ],
                4,
            )
            .expect("bounded runtime view should resolve");

        assert_eq!(plan.requested_count, 4);
        assert!(!plan.complete);
        assert_eq!(plan.views.len(), 4);
        assert_eq!(plan.views[0].source, TM_TRANSACTION_VIEW_SOURCE_QUEUE);
        assert_eq!(plan.views[0].found, true);
        assert_eq!(plan.views[0].tx_rlp, vec![0xaa, 0xbb, 0xcc]);
        assert_eq!(
            plan.views[1].source,
            TM_TRANSACTION_VIEW_SOURCE_NON_FINALIZED_SIDECAR
        );
        assert_eq!(plan.views[1].found, true);
        assert_eq!(plan.views[1].tx_rlp, vec![0x22]);
        assert_eq!(
            plan.views[2].source,
            TM_TRANSACTION_VIEW_SOURCE_RECENTLY_FINALIZED_SIDECAR
        );
        assert_eq!(plan.views[2].tx_rlp, vec![0x33]);
        assert_eq!(
            plan.views[3].source,
            TM_TRANSACTION_VIEW_SOURCE_STORAGE_PENDING
        );
        assert_eq!(plan.views[3].tx_rlp, vec![0x44]);

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_runtime_remove_non_finalized_deletes_storage_rows() {
        let temp_dir =
            unique_temp_dir("rustaxa_bridge_transaction_manager_runtime_remove_non_finalized");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let mut runtime = create_transaction_manager_runtime_from_storage(
            &storage,
            3,
            TransactionQueueConfig { max_size: 16 },
        );

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

    #[test]
    fn bridge_transaction_manager_runtime_lookup_transaction_views_with_final_chain_marks_old_finalized(
    ) {
        let temp_dir = unique_temp_dir(
            "rustaxa_bridge_transaction_manager_runtime_lookup_transaction_views_fc",
        );
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        let signing_key = SigningKey::from_slice(&[0x34u8; 32]).unwrap();
        let sender = address_from_signing_key(&signing_key);
        let sender_bytes: [u8; 20] = sender.into();
        let transaction_rlp = signed_legacy_transaction_rlp(&signing_key, 1, 2999);
        let transaction_hash = keccak256(&transaction_rlp).0;
        let pbft_key = SigningKey::from_slice(&[0x35u8; 32]).unwrap();
        let pbft_block = signed_pbft_block(&pbft_key, 1, 1000);
        storage
            .0
            .transaction()
            .write_location(H256::from(transaction_hash), 1, 0, false)
            .expect("proposal storage location should persist");
        storage
            .0
            .period()
            .write(
                1,
                &period_data_rlp_with_pbft(&pbft_block, std::slice::from_ref(&transaction_rlp)),
            )
            .expect("period data should persist");

        let final_chain = crate::final_chain::create_final_chain(
            &storage,
            1_000_000,
            0,
            vec![crate::ffi::rustaxa_ffi::GenesisAccount {
                address: sender_bytes,
                balance: U256::from(1_000_000u64).to_big_endian().to_vec(),
            }],
            vec![],
            crate::ffi::rustaxa_ffi::GenesisDposConfig {
                eligibility_balance_threshold: vec![],
                vote_eligibility_balance_step: U256::one().to_big_endian().to_vec(),
                validator_maximum_stake: U256::from(30_000u64).to_big_endian().to_vec(),
                minimum_deposit: vec![],
                commission_change_delta: 0,
                commission_change_frequency: 0,
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("final chain should initialize");

        final_chain
            .finalize_block(
                pbft_block,
                vec![crate::ffi::rustaxa_ffi::FinalizationTransaction {
                    hash: transaction_hash,
                    sender: sender_bytes,
                    receiver_found: true,
                    receiver: [0x55u8; 20],
                    nonce: 2,
                    value: U256::from(3u64).to_big_endian().to_vec(),
                    gas_price: U256::from(2u64).to_big_endian().to_vec(),
                    gas_limit: 21_000,
                    data: vec![],
                    rlp: transaction_rlp.clone(),
                }],
                vec![],
            )
            .expect("finalization should create block-scoped account snapshot");

        let runtime = create_transaction_manager_runtime_from_storage(
            &storage,
            11,
            TransactionQueueConfig { max_size: 32 },
        );
        let plan = runtime
            .transaction_manager_runtime_lookup_proposal_transaction_views(
                &final_chain,
                1,
                vec![TransactionManagerTransactionViewRequest {
                    input_index: 10,
                    hash: transaction_hash,
                }],
                0,
            )
            .expect("runtime view lookup with final-chain filtering should execute");

        assert_eq!(plan.requested_count, 1);
        assert!(plan.complete);
        assert_eq!(plan.views.len(), 1);
        assert!(plan.views[0].old_finalized);
        assert_eq!(plan.views[0].found, false);
        assert_eq!(
            plan.views[0].source,
            TM_TRANSACTION_VIEW_SOURCE_STORAGE_FINALIZED_REGULAR
        );
        assert!(plan.views[0].tx_rlp.is_empty());

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
        let mut runtime = create_transaction_manager_runtime_from_storage(
            &storage,
            3,
            TransactionQueueConfig { max_size: 16 },
        );

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
        let mut runtime = create_transaction_manager_runtime_from_storage(
            &storage,
            7,
            TransactionQueueConfig { max_size: 16 },
        );
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

        let mut runtime = create_transaction_manager_runtime_from_storage(
            &storage,
            7,
            TransactionQueueConfig { max_size: 16 },
        );
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
    fn bridge_update_finalized_transactions_status_command_report_with_final_chain_executes_periodic_purge_boundary(
    ) {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_update_finalized_status_report_fc_purge");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let sender = [9; 20];
        let final_chain = crate::final_chain::create_final_chain(
            &storage,
            1_000_000,
            1,
            vec![crate::ffi::rustaxa_ffi::GenesisAccount {
                address: sender,
                balance: vec![1],
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

        let mut runtime = create_transaction_manager_runtime_from_storage(
            &storage,
            7,
            TransactionQueueConfig { max_size: 16 },
        );
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input_for_sender(
                1, sender, 1, true,
            ))
            .expect("queue seed should succeed");

        let report =
            update_finalized_transactions_status_command_report_with_runtime_and_final_chain(
                &mut runtime,
                &final_chain,
                100,
                10,
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
    fn bridge_transaction_manager_load_stored_transactions_orders_and_classifies_sources() {
        let temp_dir =
            unique_temp_dir("rustaxa_bridge_transaction_manager_load_stored_transactions");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .0
            .transaction()
            .write(H256::from([1u8; 32]), &[0x11])
            .expect("pending transaction should persist");

        // Persist finalized location metadata and tx-by-position data for hash 2 so lookup
        // exercises finalized fallback path after non-finalized miss.
        storage
            .0
            .transaction()
            .write_location(H256::from([2u8; 32]), 8, 0, false)
            .expect("finalized location should persist");

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
            .write(8, &period_data.out().as_ref().to_vec())
            .expect("period data should persist");
        storage
            .0
            .transaction()
            .write_system(H256::from([4u8; 32]), &[0x44])
            .expect("system transaction should persist");
        storage
            .0
            .transaction()
            .write_location(H256::from([4u8; 32]), 9, 0, true)
            .expect("system finalized location should persist");

        let out = transaction_manager_load_stored_transactions_from_storage(
            &storage.0,
            vec![
                TransactionManagerStoredTransactionRequest {
                    input_index: 7,
                    hash: [2u8; 32],
                },
                TransactionManagerStoredTransactionRequest {
                    input_index: 8,
                    hash: [3u8; 32],
                },
                TransactionManagerStoredTransactionRequest {
                    input_index: 9,
                    hash: [1u8; 32],
                },
                TransactionManagerStoredTransactionRequest {
                    input_index: 10,
                    hash: [4u8; 32],
                },
            ],
        )
        .expect("transaction payload lookup should preserve order");

        let out = out
            .into_iter()
            .map(|entry| (entry.hash, entry.found, entry.source, entry.tx_rlp))
            .collect::<Vec<_>>();

        assert_eq!(out.len(), 4);
        assert_eq!(
            out[0],
            (
                [2u8; 32],
                true,
                TM_STORED_TX_SOURCE_FINALIZED_REGULAR,
                vec![0x22]
            )
        );
        assert_eq!(
            out[1],
            (
                [3u8; 32],
                false,
                TM_STORED_TX_SOURCE_MISSING,
                Vec::<u8>::new()
            )
        );
        assert_eq!(
            out[2],
            ([1u8; 32], true, TM_STORED_TX_SOURCE_PENDING, vec![0x11])
        );
        assert_eq!(
            out[3],
            (
                [4u8; 32],
                true,
                TM_STORED_TX_SOURCE_FINALIZED_SYSTEM,
                vec![0x44]
            )
        );

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

        let mut runtime = create_transaction_manager_runtime_from_storage(
            &storage,
            4,
            TransactionQueueConfig { max_size: 16 },
        );

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
    fn bridge_transaction_manager_runtime_queue_block_finalized_returns_expired_hashes() {
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 16 });
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
    fn runtime_queue_account_nonce_facts_from_final_chain_maps_found_and_missing_accounts() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_queue_nonce_facts");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let genesis_account = crate::ffi::rustaxa_ffi::GenesisAccount {
            address: [1; 20],
            balance: vec![1],
        };
        let final_chain = crate::final_chain::create_final_chain(
            &storage,
            1_000_000,
            1,
            vec![genesis_account],
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
        let facts = runtime_queue_account_nonce_facts_from_final_chain(
            &final_chain,
            vec![H160::from([1; 20]), H160::from([2; 20])],
        )
        .expect("fact collection should succeed");

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
    fn bridge_transaction_manager_runtime_queue_cleanup_with_final_chain_collects_facts_and_runs() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_queue_cleanup_fc");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let sender = [7; 20];
        let final_chain = crate::final_chain::create_final_chain(
            &storage,
            1_000_000,
            1,
            vec![crate::ffi::rustaxa_ffi::GenesisAccount {
                address: sender,
                balance: vec![1],
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
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 32 });
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
            .transaction_manager_runtime_queue_cleanup_with_final_chain(&final_chain, false, 20)
            .expect("cleanup with final chain should succeed");

        assert_eq!(cleanup.non_proposable_expired.removed_count, 0);
        assert!(
            cleanup.finalized_account_purged.removed_count <= 2,
            "purge should only affect proposable sender entries"
        );
        assert!(runtime.transaction_manager_runtime_queue_contains(&[3; 32]));

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_runtime_pack_session_round_trips_and_clears() {
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        let signing_key = SigningKey::from_slice(&[0x41u8; 32]).unwrap();
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
                tx_rlp: tx_rlp.clone(),
                proposable: true,
                last_block_number: 0,
            })
            .expect("queue insert should succeed");

        runtime
            .transaction_manager_runtime_pack_begin_sharded(63_000, 21_000, 7, 0, 10, 1, 0, 1)
            .expect("pack session should begin");

        let step = runtime
            .transaction_manager_runtime_pack_request_next()
            .expect("candidate request should succeed");
        assert!(step.request_estimate);
        let candidate = step.candidate;
        assert!(candidate.found);
        assert_eq!(candidate.hash, envelope.hash.0);
        assert_eq!(candidate.sender, sender.0);
        assert_eq!(candidate.nonce, envelope.nonce.to_big_endian());
        assert_eq!(candidate.gas_price, envelope.gas_price.to_big_endian());
        assert_eq!(candidate.declared_gas, 21_000);
        assert_eq!(candidate.gas, 21_000);
        assert!(candidate.receiver_found);
        assert_eq!(candidate.receiver, H160::from([0x44u8; 20]).0);
        assert_eq!(candidate.value, U256::from(3u64).to_big_endian());
        assert!(candidate.data.is_empty());

        let final_step = runtime
            .transaction_manager_runtime_pack_record_estimate_step(
                TransactionPackSessionEstimateInput {
                    hash: envelope.hash.0,
                    gas_used: 42_000,
                    last_block_number: 10,
                    result_rlp: vec![0xc0],
                },
            )
            .expect("record estimate should succeed");
        assert!(!final_step.request_estimate);
        assert!(final_step.stopped);
        assert_eq!(final_step.selected_transactions.len(), 1);
        assert_eq!(final_step.selected_transactions[0].hash, envelope.hash.0);
        assert_eq!(final_step.selected_transactions[0].gas_used, 42_000);
        assert!(!runtime.transaction_manager_runtime_pack_abort());
    }

    #[test]
    fn bridge_transaction_manager_runtime_pack_request_next_drives_estimate_loop() {
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        let signing_key = SigningKey::from_slice(&[0x42u8; 32]).unwrap();
        let sender = address_from_signing_key(&signing_key);
        let first_rlp = signed_legacy_transaction_rlp(&signing_key, 1, 2999);
        let first_envelope = LegacyTransactionEnvelope::decode(&first_rlp).unwrap();
        let second_rlp = signed_legacy_transaction_rlp(&signing_key, 2, 2999);
        let second_envelope = LegacyTransactionEnvelope::decode(&second_rlp).unwrap();
        runtime
            .transaction_manager_runtime_queue_insert(TransactionQueueInsertInput {
                hash: first_envelope.hash.0,
                sender: sender.0,
                nonce: first_envelope.nonce.to_big_endian(),
                gas_price: first_envelope.gas_price.to_big_endian(),
                gas: first_envelope.gas,
                data_size: first_envelope.data.len(),
                tx_rlp: first_rlp,
                proposable: true,
                last_block_number: 0,
            })
            .expect("proposable insert should succeed");
        runtime
            .transaction_manager_runtime_queue_insert(TransactionQueueInsertInput {
                hash: second_envelope.hash.0,
                sender: sender.0,
                nonce: second_envelope.nonce.to_big_endian(),
                gas_price: second_envelope.gas_price.to_big_endian(),
                gas: second_envelope.gas,
                data_size: second_envelope.data.len(),
                tx_rlp: second_rlp,
                proposable: true,
                last_block_number: 0,
            })
            .expect("proposable insert should succeed");

        runtime
            .transaction_manager_runtime_pack_begin_sharded(63_000, 21_000, 7, 0, 10, 1, 0, 1)
            .expect("pack session should begin");

        let first_step = runtime
            .transaction_manager_runtime_pack_request_next()
            .expect("first request should be emitted");
        assert!(first_step.request_estimate);
        assert!(first_step.candidate.found);
        let first_hash = first_step.candidate.hash;

        let second_step = runtime
            .transaction_manager_runtime_pack_record_estimate_step(
                TransactionPackSessionEstimateInput {
                    hash: first_hash,
                    gas_used: 30_000,
                    last_block_number: 10,
                    result_rlp: vec![0xc0],
                },
            )
            .expect("first estimate should return next request");
        assert!(second_step.request_estimate);
        let second_hash = second_step.candidate.hash;

        let final_step = runtime
            .transaction_manager_runtime_pack_record_estimate_step(
                TransactionPackSessionEstimateInput {
                    hash: second_hash,
                    gas_used: 20_000,
                    last_block_number: 11,
                    result_rlp: vec![0xc0],
                },
            )
            .expect("loop should finalize after last candidate");
        assert!(!final_step.request_estimate);
        assert_eq!(final_step.selected_transactions.len(), 1);
        assert_eq!(final_step.selected_transactions[0].hash, first_hash);
        assert_eq!(final_step.selected_transactions[0].gas_used, 30_000);
        assert_eq!(final_step.demoted_hashes.len(), 1);
        assert_eq!(final_step.demoted_hashes[0].hash, second_hash);

        assert!(!runtime.transaction_manager_runtime_pack_abort());
        runtime
            .transaction_manager_runtime_pack_begin_sharded(21_000, 21_000, 7, 0, 10, 1, 0, 1)
            .expect("completed step session should be cleared");
    }

    #[test]
    fn bridge_transaction_manager_runtime_pack_session_filters_candidate_shards() {
        fn legacy_sender_shard(sender: H160, proposal_period: u64, total_shards: u16) -> u16 {
            let prefix = legacy_transaction_shard_sender_prefix(sender);
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
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
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

        runtime
            .transaction_manager_runtime_pack_begin_sharded(
                63_000,
                21_000,
                proposal_period,
                0,
                10,
                total_shards,
                first_shard,
                10,
            )
            .expect("sharded pack session should begin");

        let first_step = runtime
            .transaction_manager_runtime_pack_request_next()
            .expect("matching-shard candidate should be requested");
        assert!(first_step.request_estimate);
        assert_eq!(first_step.candidate.hash, first_envelope.hash.0);

        let final_step = runtime
            .transaction_manager_runtime_pack_record_estimate_step(
                TransactionPackSessionEstimateInput {
                    hash: first_step.candidate.hash,
                    gas_used: 30_000,
                    last_block_number: 10,
                    result_rlp: vec![0xc0],
                },
            )
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
    fn bridge_transaction_manager_shard_prefix_matches_legacy_five_byte_parse() {
        let sender = H160::from([
            0x01, 0x23, 0x45, 0x67, 0x89, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x10, 0x20, 0x30,
            0x40, 0x50, 0x60, 0x70, 0x80, 0x90,
        ]);

        assert_eq!(
            legacy_transaction_shard_sender_prefix(sender),
            0x01_23_45_67_89
        );
    }

    #[test]
    fn bridge_transaction_manager_runtime_pack_session_consumes_declared_and_cached_gas() {
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
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

        runtime
            .transaction_manager_runtime_pack_begin_sharded(63_000, 21_000, 7, 25_000, 10, 1, 0, 1)
            .expect("pack session should begin");

        let final_step = runtime
            .transaction_manager_runtime_pack_request_next()
            .expect("declared/cache paths should finish without C++ estimate");
        assert!(!final_step.request_estimate);
        assert_eq!(final_step.selected_transactions.len(), 2);
        assert_eq!(
            final_step.selected_transactions[0].hash,
            first_envelope.hash.0
        );
        assert_eq!(final_step.selected_transactions[0].gas_used, 21_000);
        assert_eq!(
            final_step.selected_transactions[1].hash,
            second_envelope.hash.0
        );
        assert_eq!(final_step.selected_transactions[1].gas_used, 21_000);
        assert!(!runtime.transaction_manager_runtime_pack_abort());

        let mut cached_runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 100 });
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
        cached_runtime
            .transaction_manager_runtime_pack_begin_sharded(63_000, 21_000, 7, 0, 10, 1, 0, 1)
            .expect("cached pack session should begin");
        let cached_step = cached_runtime
            .transaction_manager_runtime_pack_request_next()
            .expect("cached path should finish without C++ estimate");
        assert!(!cached_step.request_estimate);
        assert_eq!(cached_step.selected_transactions.len(), 1);
        assert_eq!(
            cached_step.selected_transactions[0].hash,
            second_envelope.hash.0
        );
        assert_eq!(cached_step.selected_transactions[0].gas_used, 30_000);
        assert!(!cached_runtime.transaction_manager_runtime_pack_abort());
    }

    #[test]
    fn bridge_transaction_manager_runtime_plans_and_caches_gas_estimation() {
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 100 });

        let small = runtime
            .transaction_manager_runtime_plan_gas_estimation(TransactionManagerGasEstimationFact {
                hash: [1; 32],
                declared_gas: 21_000,
                proposal_period: 5,
                estimate_gas_limit: 200_000,
            })
            .expect("small gas estimation plan should succeed");
        assert!(small.use_declared_gas);
        assert!(!small.cache_hit);
        assert!(!small.requires_evm_call);
        assert_eq!(small.gas_used, 21_000);

        let miss = runtime
            .transaction_manager_runtime_plan_gas_estimation(TransactionManagerGasEstimationFact {
                hash: [2; 32],
                declared_gas: 300_000,
                proposal_period: 5,
                estimate_gas_limit: 200_000,
            })
            .expect("cache miss plan should succeed");
        assert!(!miss.use_declared_gas);
        assert!(!miss.cache_hit);
        assert!(miss.requires_evm_call);

        assert!(runtime
            .transaction_manager_runtime_store_gas_estimation(
                TransactionManagerGasEstimationResult {
                    hash: [2; 32],
                    proposal_period: 5,
                    gas_used: 44_000,
                    result_rlp: vec![0xc0],
                },
            )
            .expect("cache store should succeed"));

        let hit = runtime
            .transaction_manager_runtime_plan_gas_estimation(TransactionManagerGasEstimationFact {
                hash: [2; 32],
                declared_gas: 300_000,
                proposal_period: 5,
                estimate_gas_limit: 200_000,
            })
            .expect("cache hit plan should succeed");
        assert!(!hit.use_declared_gas);
        assert!(hit.cache_hit);
        assert!(!hit.requires_evm_call);
        assert_eq!(hit.gas_used, 44_000);
        assert_eq!(hit.result_rlp, vec![0xc0]);

        let different_period = runtime
            .transaction_manager_runtime_plan_gas_estimation(TransactionManagerGasEstimationFact {
                hash: [2; 32],
                declared_gas: 300_000,
                proposal_period: 6,
                estimate_gas_limit: 200_000,
            })
            .expect("different period plan should succeed");
        assert!(different_period.requires_evm_call);
    }
}
