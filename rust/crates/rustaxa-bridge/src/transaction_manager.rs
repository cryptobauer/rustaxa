//! CXX bridge wrappers for Rust `TransactionManager` decision boundaries.
//!
//! The bridge exposes:
//! - a short-lived planner used while one DAG proposal is being packed
//! - a storage-complete planner for `TransactionManager::saveTransactionsFromDagBlock`
//! - an opaque live sidecar handle for non-finalized and recently-finalized payloads
//!
//! C++ supplies transaction metadata, RLP payloads, and queue-known facts. Rust owns
//! deterministic planning, latest-state FinalChain account fact sourcing, storage
//! mutations routed through Rust storage, live transaction count authority, admission
//! status mapping, and sidecar membership/RLP bytes, but not C++ `Transaction`
//! pointers or gas estimation.

use crate::ffi::rustaxa_ffi::{
    DagTransactionSaveAccepted, DagTransactionSaveFact, DagTransactionSaveOutcome,
    DagTransactionSaveRuntimeFact, DagTransactionSaveSidecarFact, FinalizedTransactionFilterPlan,
    FinalizedTransactionStatusAction, FinalizedTransactionStatusFact,
    FinalizedTransactionStatusPlan, FinalizedTransactionStatusSidecarFact,
    NonFinalizedTransactionPayload, TransactionManagerAdmissionCommandReport,
    TransactionManagerAdmissionResult, TransactionManagerDagSaveCommandReport,
    TransactionManagerFilterAction, TransactionManagerFinalizedFilterFact,
    TransactionManagerFinalizedStatusCommandReport, TransactionManagerGasEstimationFact,
    TransactionManagerGasEstimationPlan, TransactionManagerGasEstimationResult,
    TransactionManagerHashCommand, TransactionManagerInsertTransactionFact,
    TransactionManagerInsertTransactionOutcome, TransactionManagerPublicAdmissionCommandReport,
    TransactionManagerPublicInsertResult, TransactionManagerRecoveryEntry,
    TransactionManagerRuntimeAdmissionOutcome, TransactionManagerRuntimeQueueCleanupPlan,
    TransactionManagerRuntimeValidatedInsertOutcome, TransactionManagerSidecarInsertInput,
    TransactionManagerSidecarKnownFact, TransactionManagerSidecarLookup,
    TransactionManagerSidecarLookupPlan, TransactionManagerSidecarLookupRequest,
    TransactionManagerSidecarRecoveryInsertInput, TransactionManagerSidecarTransitionInput,
    TransactionManagerStoredTransactionLookup, TransactionManagerStoredTransactionRequest,
    TransactionManagerTransactionView, TransactionManagerTransactionViewPlan,
    TransactionManagerTransactionViewRequest, TransactionManagerValidatedInsertFact,
    TransactionManagerValidatedInsertPlan, TransactionManagerValidatedInsertRuntimeFact,
    TransactionManagerValidatedInsertSidecarFact, TransactionManagerVerifyNotFinalizedFact,
    TransactionManagerVerifyNotFinalizedOutcome, TransactionManagerVerifyNotFinalizedRuntimeFact,
    TransactionManagerVerifyNotFinalizedSidecarFact, TransactionManagerVerifyTransactionFact,
    TransactionManagerVerifyTransactionOutcome, TransactionPackEstimateOutcome,
    TransactionPackSelectedTransaction, TransactionPackSessionCandidate,
    TransactionPackSessionEstimateInput, TransactionPackSessionStep,
    TransactionQueueAccountNonceFact as BridgeTransactionQueueAccountNonceFact,
    TransactionQueueAddress, TransactionQueueConfig, TransactionQueueHash,
    TransactionQueueInsertInput, TransactionQueueInsertOutcome, TransactionQueuePurgePlan,
    TransactionQueueStoredTransaction, TransactionQueueTransactionGroup,
};
use crate::ffi::{
    BridgeFinalChain, BridgeStorage, BridgeTransactionManagerAdmissionExecution,
    BridgeTransactionManagerRuntime, TransactionManagerRuntimePackSession,
};
use crate::transaction::legacy_transaction_inspection_from_bytes;
use anyhow::{anyhow, ensure, Context, Result};
use ethereum_types::{H160, H256, U256};
use rustaxa_consensus::transaction_manager::{
    plan_exclude_finalized_transactions as plan_exclude_finalized_transactions_from_storage,
    plan_finalized_transactions_status, plan_insert_transaction, plan_transactions_from_dag_block,
    plan_validated_insert,
    plan_verify_not_finalized_transactions as plan_verify_not_finalized_transactions_from_storage,
    plan_verify_transaction, DagTransactionSaveFact as ConsensusDagTransactionSaveFact,
    DagTransactionSavePayload,
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
    VerifyNotFinalizedTransactionFact as ConsensusVerifyNotFinalizedTransactionFact,
    VerifyNotFinalizedTransactionPlan as ConsensusVerifyNotFinalizedTransactionPlan,
};
use rustaxa_consensus::transaction_queue::{
    TransactionQueue, TransactionQueueAccountNonceFact, TransactionQueueDemoteStatus,
    TransactionQueueEntry, TransactionQueueInsertStatus, TransactionQueuePurgeOutcome,
};
use rustaxa_types::LegacyTransactionEnvelope;
use std::time::{Duration, Instant};

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
const TM_VALIDATED_INSERT_QUEUE_ACTION_NONE: u8 = 0;
const TM_VALIDATED_INSERT_QUEUE_ACTION_INSERT_PROPOSABLE: u8 = 1;
const TM_VALIDATED_INSERT_QUEUE_ACTION_INSERT_NON_PROPOSABLE: u8 = 2;
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
    TransactionManagerAdmissionCommandReport {
        inserted_hash_found: outcome.inserted_hash_found,
        inserted_hash: outcome.inserted_hash,
        transaction_added_hash_found: outcome.emit_transaction_added && outcome.inserted_hash_found,
        transaction_added_hash: outcome.inserted_hash,
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

fn runtime_queue_account_nonce_facts_from_bridge(
    facts: Vec<BridgeTransactionQueueAccountNonceFact>,
) -> Vec<TransactionQueueAccountNonceFact> {
    facts
        .into_iter()
        .map(|fact| TransactionQueueAccountNonceFact {
            sender: H160::from(fact.sender),
            account_found: fact.account_found,
            account_nonce: U256::from_big_endian(&fact.account_nonce),
        })
        .collect::<Vec<_>>()
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

fn final_chain_account_lookup(
    final_chain: &BridgeFinalChain,
    sender: &[u8; 20],
) -> Result<(bool, U256, U256)> {
    let lookup = final_chain
        .get_account(sender)
        .context("TM_FINAL_CHAIN_ACCOUNT_LOOKUP_FAILED")?;
    let account_nonce = U256::from(lookup.nonce);
    let account_balance = if lookup.found {
        U256::from_big_endian(&lookup.balance)
    } else {
        U256::zero()
    };
    Ok((lookup.found, account_nonce, account_balance))
}

fn final_chain_transaction_period_lookup(
    final_chain: &BridgeFinalChain,
    hash: &[u8; 32],
) -> Result<Option<u64>> {
    let location = final_chain
        .get_transaction_location(hash)
        .context("TM_FINAL_CHAIN_TRANSACTION_LOCATION_LOOKUP_FAILED")?;
    if location.is_empty() {
        return Ok(None);
    }
    let rlp = rlp::Rlp::new(&location);
    let period = rlp
        .val_at::<u64>(0)
        .context("TM_FINAL_CHAIN_TRANSACTION_LOCATION_PERIOD")?;
    Ok(Some(period))
}

fn storage_transaction_period_lookup(
    storage: &BridgeStorage,
    hash: &[u8; 32],
) -> Result<Option<u64>> {
    let location = storage
        .get_transaction_location(hash)
        .context("TM_STORAGE_TRANSACTION_LOCATION_LOOKUP_FAILED")?;
    if location.is_empty() {
        return Ok(None);
    }
    let rlp = rlp::Rlp::new(&location);
    let period = rlp
        .val_at::<u64>(0)
        .context("TM_STORAGE_TRANSACTION_LOCATION_PERIOD")?;
    Ok(Some(period))
}

/// Plans and persists accepted transactions for one incoming DAG block.
///
/// This call is storage-complete: accepted payloads are written with one atomic
/// batch through `save_non_finalized_transactions`, and `target_transaction_count`
/// is returned for C++ cache-state bookkeeping.
pub fn save_transactions_from_dag_block(
    storage: &BridgeStorage,
    current_transaction_count: u64,
    facts: Vec<DagTransactionSaveFact>,
) -> Result<DagTransactionSaveOutcome> {
    let plan = plan_transactions_from_dag_block(
        facts
            .into_iter()
            .map(consensus_fact_from_ffi_fact)
            .collect(),
        current_transaction_count,
        |hash| {
            storage
                .0
                .transaction()
                .finalized(hash)
                .context("TM_DAG_TX_FINALIZED_LOOKUP_FAILED")
        },
    )?;

    let mut accepted: Vec<DagTransactionSaveAccepted> =
        Vec::with_capacity(plan.accepted_transactions.len());
    let mut accepted_payloads: Vec<NonFinalizedTransactionPayload> =
        Vec::with_capacity(plan.accepted_transactions.len());

    for DagTransactionSavePayload {
        input_index,
        hash,
        trx_rlp,
    } in plan.accepted_transactions
    {
        accepted.push(DagTransactionSaveAccepted {
            input_index,
            hash: hash.0,
            erased_from_queue: false,
        });
        accepted_payloads.push(NonFinalizedTransactionPayload {
            hash: hash.0,
            trx_rlp,
        });
    }

    if !accepted_payloads.is_empty() {
        storage
            .save_non_finalized_transactions(accepted_payloads, plan.target_transaction_count)?;
    }

    Ok(DagTransactionSaveOutcome {
        accepted,
        target_transaction_count: plan.target_transaction_count,
    })
}

/// Plans and persists accepted DAG-block transactions while Rust owns live sidecars.
///
/// Sidecar membership is read from `sidecar`; C++ supplies only transaction
/// payloads and FinalChain nonce facts. New runtime routes source latest account
/// facts from `BridgeFinalChain`, while this fact-driven API remains for parity
/// and focused bridge tests. The Rust live sidecar is mutated only after the
/// storage batch succeeds.
pub fn save_transactions_from_dag_block_with_sidecar(
    sidecar: &mut BridgeTransactionManagerSidecar,
    storage: &BridgeStorage,
    facts: Vec<DagTransactionSaveSidecarFact>,
) -> Result<DagTransactionSaveOutcome> {
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
                    in_non_finalized_cache: sidecar.0.contains_non_finalized(hash),
                    in_recently_finalized_cache: sidecar.0.contains_recently_finalized(hash),
                }
            })
            .collect(),
        sidecar.0.transaction_count(),
        |hash| {
            storage
                .0
                .transaction()
                .finalized(hash)
                .context("TM_DAG_TX_FINALIZED_LOOKUP_FAILED")
        },
    )?;

    let mut accepted: Vec<DagTransactionSaveAccepted> =
        Vec::with_capacity(plan.accepted_transactions.len());
    let mut accepted_payloads: Vec<NonFinalizedTransactionPayload> =
        Vec::with_capacity(plan.accepted_transactions.len());

    for payload in &plan.accepted_transactions {
        accepted.push(DagTransactionSaveAccepted {
            input_index: payload.input_index,
            hash: payload.hash.0,
            erased_from_queue: false,
        });
        accepted_payloads.push(NonFinalizedTransactionPayload {
            hash: payload.hash.0,
            trx_rlp: payload.trx_rlp.clone(),
        });
    }

    if !accepted_payloads.is_empty() {
        storage
            .save_non_finalized_transactions(accepted_payloads, plan.target_transaction_count)?;
    }

    for payload in plan.accepted_transactions {
        sidecar
            .0
            .insert_non_finalized(payload.hash, payload.trx_rlp)
            .context("TM_SIDECAR_DAG_TX_INSERT")?;
    }
    sidecar
        .0
        .set_transaction_count(plan.target_transaction_count);

    Ok(DagTransactionSaveOutcome {
        accepted,
        target_transaction_count: plan.target_transaction_count,
    })
}

/// Executes one runtime admission pass and returns explicit storage and live-state effects.
pub fn transaction_manager_runtime_execute_admission(
    runtime: &BridgeTransactionManagerRuntime,
    storage: &BridgeStorage,
    facts: Vec<DagTransactionSaveSidecarFact>,
) -> Result<Box<BridgeTransactionManagerAdmissionExecution>> {
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
        |hash| {
            storage
                .0
                .transaction()
                .finalized(hash)
                .context("TM_DAG_TX_FINALIZED_LOOKUP_FAILED")
        },
    )?;

    let mut accepted: Vec<DagTransactionSaveAccepted> =
        Vec::with_capacity(plan.accepted_transactions.len());
    let mut accepted_payloads: Vec<NonFinalizedTransactionPayload> =
        Vec::with_capacity(plan.accepted_transactions.len());

    for payload in &plan.accepted_transactions {
        accepted.push(DagTransactionSaveAccepted {
            input_index: payload.input_index,
            hash: payload.hash.0,
            erased_from_queue: false,
        });
        accepted_payloads.push(NonFinalizedTransactionPayload {
            hash: payload.hash.0,
            trx_rlp: payload.trx_rlp.clone(),
        });
    }

    Ok(Box::new(BridgeTransactionManagerAdmissionExecution {
        accepted,
        accepted_payloads,
        target_transaction_count: plan.target_transaction_count,
    }))
}

/// Commits one runtime admission execution script with storage-first ordering.
#[allow(clippy::boxed_local)]
pub fn transaction_manager_runtime_commit_admission(
    runtime: &mut BridgeTransactionManagerRuntime,
    storage: &BridgeStorage,
    execution: Box<BridgeTransactionManagerAdmissionExecution>,
) -> Result<DagTransactionSaveOutcome> {
    let BridgeTransactionManagerAdmissionExecution {
        accepted,
        accepted_payloads,
        target_transaction_count,
    } = *execution;

    if !accepted_payloads.is_empty() {
        let storage_payloads: Vec<NonFinalizedTransactionPayload> = accepted_payloads
            .iter()
            .map(|payload| NonFinalizedTransactionPayload {
                hash: payload.hash,
                trx_rlp: payload.trx_rlp.clone(),
            })
            .collect();
        storage.save_non_finalized_transactions(storage_payloads, target_transaction_count)?;
    }

    let mut accepted = accepted;
    for accepted_entry in &mut accepted {
        accepted_entry.erased_from_queue = runtime.queue.erase(H256::from(accepted_entry.hash));
    }

    for payload in accepted_payloads {
        runtime
            .sidecar
            .insert_non_finalized(H256::from(payload.hash), payload.trx_rlp)
            .context("TM_RUNTIME_DAG_TX_INSERT")?;
    }
    runtime
        .sidecar
        .set_transaction_count(target_transaction_count);

    Ok(DagTransactionSaveOutcome {
        accepted,
        target_transaction_count,
    })
}

/// Plans and persists accepted DAG-block transactions through the Rust manager runtime.
pub fn save_transactions_from_dag_block_with_runtime(
    runtime: &mut BridgeTransactionManagerRuntime,
    storage: &BridgeStorage,
    facts: Vec<DagTransactionSaveSidecarFact>,
) -> Result<DagTransactionSaveOutcome> {
    let execution = transaction_manager_runtime_execute_admission(runtime, storage, facts)?;
    transaction_manager_runtime_commit_admission(runtime, storage, execution)
}

/// Plans and persists accepted DAG-block transactions through the Rust runtime
/// with sender account nonces sourced from latest FinalChain state.
pub fn save_transactions_from_dag_block_with_runtime_and_final_chain(
    runtime: &mut BridgeTransactionManagerRuntime,
    storage: &BridgeStorage,
    final_chain: &BridgeFinalChain,
    facts: Vec<DagTransactionSaveRuntimeFact>,
) -> Result<DagTransactionSaveOutcome> {
    let sidecar_facts = facts
        .into_iter()
        .map(|fact| {
            let (_, account_nonce, _) = final_chain_account_lookup(final_chain, &fact.sender)?;
            Ok(DagTransactionSaveSidecarFact {
                input_index: fact.input_index,
                hash: fact.hash,
                trx_rlp: fact.trx_rlp,
                transaction_nonce: fact.transaction_nonce,
                sender_account_nonce: account_nonce.to_big_endian(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    save_transactions_from_dag_block_with_runtime(runtime, storage, sidecar_facts)
}

/// Returns transaction-manager DAG persistence as typed command actions.
pub fn save_transactions_from_dag_block_command_report_with_runtime_and_final_chain(
    runtime: &mut BridgeTransactionManagerRuntime,
    storage: &BridgeStorage,
    final_chain: &BridgeFinalChain,
    facts: Vec<DagTransactionSaveRuntimeFact>,
) -> Result<TransactionManagerDagSaveCommandReport> {
    let outcome = save_transactions_from_dag_block_with_runtime_and_final_chain(
        runtime,
        storage,
        final_chain,
        facts,
    )?;
    Ok(dag_save_command_report(&outcome))
}

/// Plans finalized-transaction status transitions for one period.
///
/// This call is storage-bound for the `StatusDbField::TrxCount` counter only.
/// The counter persists only when the finalized transaction list is non-empty,
/// matching existing Rust-side storage behavior for conditional status updates.
pub fn update_finalized_transactions_status(
    storage: &BridgeStorage,
    period: u64,
    retention_window: u64,
    current_transaction_count: u64,
    facts: Vec<FinalizedTransactionStatusFact>,
) -> Result<FinalizedTransactionStatusPlan> {
    let plan: ConsensusFinalizedTransactionStatusPlan = plan_finalized_transactions_status(
        facts
            .into_iter()
            .map(consensus_finalized_status_fact_from_ffi_fact)
            .collect(),
        current_transaction_count,
        period,
        retention_window,
    )?;

    if !plan.accepted_transactions.is_empty() {
        storage
            .save_status_field(
                rustaxa_storage::StatusField::TrxCount as u8,
                plan.target_transaction_count,
            )
            .context("TM_FINALIZED_STATUS_TRXCOUNT_WRITE")?;
    }

    Ok(FinalizedTransactionStatusPlan {
        accepted: plan
            .accepted_transactions
            .into_iter()
            .map(|action| FinalizedTransactionStatusAction {
                input_index: action.input_index,
                hash: action.hash.0,
                removed_non_finalized: action.removed_non_finalized,
                mark_transaction_known: true,
                erase_from_queue: true,
                erased_from_queue: false,
            })
            .collect(),
        target_transaction_count: plan.target_transaction_count,
        stale_period: plan.stale_period.unwrap_or(0),
        has_stale_period: plan.stale_period.is_some(),
        purge_transaction_queue: plan.purge_transactions,
    })
}

/// Plans finalized transaction status updates while Rust owns live sidecars.
///
/// Rust computes non-finalized membership internally, persists `TrxCount`
/// before live sidecar mutation, evicts stale recently-finalized entries, and
/// stores current-period finalized payloads for later C++ materialization.
pub fn update_finalized_transactions_status_with_sidecar(
    sidecar: &mut BridgeTransactionManagerSidecar,
    storage: &BridgeStorage,
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
                in_non_finalized_cache: sidecar.0.contains_non_finalized(hash),
            }
        })
        .collect();

    let plan: ConsensusFinalizedTransactionStatusPlan = plan_finalized_transactions_status(
        consensus_facts,
        sidecar.0.transaction_count(),
        period,
        retention_window,
    )?;

    if !plan.accepted_transactions.is_empty() {
        storage
            .save_status_field(
                rustaxa_storage::StatusField::TrxCount as u8,
                plan.target_transaction_count,
            )
            .context("TM_FINALIZED_STATUS_TRXCOUNT_WRITE")?;
    }

    if let Some(stale_period) = plan.stale_period {
        sidecar
            .0
            .evict_recently_finalized_stale_period(stale_period);
    }

    for action in &plan.accepted_transactions {
        let fact = facts
            .get(action.input_index as usize)
            .context("TM_SIDECAR_FINALIZED_STATUS_INPUT_INDEX")?;
        let hash = H256::from(fact.hash);
        ensure!(
            hash == action.hash,
            "TM_SIDECAR_FINALIZED_STATUS_HASH_MISMATCH"
        );
        sidecar
            .0
            .insert_recently_finalized(period, hash, fact.trx_rlp.clone())
            .context("TM_SIDECAR_FINALIZED_STATUS_INSERT")?;
    }
    sidecar
        .0
        .set_transaction_count(plan.target_transaction_count);

    Ok(FinalizedTransactionStatusPlan {
        accepted: plan
            .accepted_transactions
            .into_iter()
            .map(|action| FinalizedTransactionStatusAction {
                input_index: action.input_index,
                hash: action.hash.0,
                removed_non_finalized: action.removed_non_finalized,
                mark_transaction_known: true,
                erase_from_queue: true,
                erased_from_queue: false,
            })
            .collect(),
        target_transaction_count: plan.target_transaction_count,
        stale_period: plan.stale_period.unwrap_or(0),
        has_stale_period: plan.stale_period.is_some(),
        purge_transaction_queue: plan.purge_transactions,
    })
}

/// Plans and applies finalized transaction status updates through the Rust manager runtime.
///
/// Rust persists count changes before mutating live runtime state. Once storage
/// succeeds, the runtime evicts stale recent-finalized sidecars, inserts current
/// finalized payloads, marks queue-known membership, erases matching queued
/// payloads, and advances the authoritative transaction count.
pub fn update_finalized_transactions_status_with_runtime(
    runtime: &mut BridgeTransactionManagerRuntime,
    storage: &BridgeStorage,
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
        storage
            .save_status_field(
                rustaxa_storage::StatusField::TrxCount as u8,
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
            input_index: action.input_index,
            hash: action.hash.0,
            removed_non_finalized: action.removed_non_finalized,
            mark_transaction_known: true,
            erase_from_queue: true,
            erased_from_queue,
        });
    }
    runtime
        .sidecar
        .set_transaction_count(plan.target_transaction_count);

    Ok(FinalizedTransactionStatusPlan {
        accepted,
        target_transaction_count: plan.target_transaction_count,
        stale_period: plan.stale_period.unwrap_or(0),
        has_stale_period: plan.stale_period.is_some(),
        purge_transaction_queue: plan.purge_transactions,
    })
}

/// Applies finalized-transaction status changes and returns typed command actions.
pub fn update_finalized_transactions_status_command_report_with_runtime(
    runtime: &mut BridgeTransactionManagerRuntime,
    storage: &BridgeStorage,
    period: u64,
    retention_window: u64,
    facts: Vec<FinalizedTransactionStatusSidecarFact>,
) -> Result<TransactionManagerFinalizedStatusCommandReport> {
    let outcome = update_finalized_transactions_status_with_runtime(
        runtime,
        storage,
        period,
        retention_window,
        facts,
    )?;
    Ok(finalized_status_command_report(&outcome))
}

/// Applies finalized status changes plus queue purge and returns typed command actions.
pub fn update_finalized_transactions_status_command_report_with_runtime_and_final_chain(
    runtime: &mut BridgeTransactionManagerRuntime,
    storage: &BridgeStorage,
    final_chain: &BridgeFinalChain,
    period: u64,
    retention_window: u64,
    facts: Vec<FinalizedTransactionStatusSidecarFact>,
) -> Result<TransactionManagerFinalizedStatusCommandReport> {
    let outcome = update_finalized_transactions_status_with_runtime(
        runtime,
        storage,
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

/// Builds a deterministic admission plan for C++ insertion pre-checks.
pub fn transaction_manager_insert_transaction(
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

/// Builds a deterministic admission plan for C++ insertion pre-checks using Rust-owned sidecars.
///
/// `fact.hash_known` is interpreted as the queue-known fact; Rust folds this with
/// sidecar membership to make one known/admission decision.
pub fn transaction_manager_insert_transaction_with_sidecar(
    sidecar: &BridgeTransactionManagerSidecar,
    fact: TransactionManagerInsertTransactionFact,
) -> Result<TransactionManagerInsertTransactionOutcome> {
    let tx_hash = H256::from(fact.tx_hash);
    let hash_known = sidecar
        .0
        .is_transaction_known(TransactionManagerKnownFact {
            hash: tx_hash,
            queue_known: fact.hash_known,
        })
        .context("TM_INSERT_WITH_SIDECAR_KNOWN_CHECK_FAILED")?;

    transaction_manager_insert_transaction(TransactionManagerInsertTransactionFact {
        hash_known,
        ..fact
    })
}

/// Builds a deterministic admission plan using Rust-owned runtime queue and sidecar state.
pub fn transaction_manager_insert_transaction_with_runtime(
    runtime: &BridgeTransactionManagerRuntime,
    fact: TransactionManagerInsertTransactionFact,
) -> Result<TransactionManagerInsertTransactionOutcome> {
    let _ = runtime;
    transaction_manager_insert_transaction(fact)
}

/// Builds a deterministic pre-mutation plan for C++ live queue insertion.
pub fn transaction_manager_plan_validated_insert(
    fact: TransactionManagerValidatedInsertFact,
) -> Result<TransactionManagerValidatedInsertPlan> {
    let plan = plan_validated_insert(consensus_validated_insert_fact_from_ffi_fact(fact))?;
    Ok(TransactionManagerValidatedInsertPlan {
        status: queue_status_to_ffi(plan.status),
        queue_action: if !plan.should_insert_queue {
            TM_VALIDATED_INSERT_QUEUE_ACTION_NONE
        } else if plan.queue_proposable {
            TM_VALIDATED_INSERT_QUEUE_ACTION_INSERT_PROPOSABLE
        } else {
            TM_VALIDATED_INSERT_QUEUE_ACTION_INSERT_NON_PROPOSABLE
        },
        emit_transaction_added: plan.emit_transaction_added,
    })
}

/// Builds a deterministic pre-mutation plan using Rust-owned sidecar membership.
pub fn transaction_manager_plan_validated_insert_with_sidecar(
    sidecar: &BridgeTransactionManagerSidecar,
    fact: TransactionManagerValidatedInsertSidecarFact,
) -> Result<TransactionManagerValidatedInsertPlan> {
    let hash = H256::from(fact.tx_hash);
    let plan = plan_validated_insert(ConsensusTransactionManagerValidatedInsertFact {
        tx_hash: hash,
        transaction_nonce: U256::from_big_endian(&fact.transaction_nonce),
        transaction_cost: U256::from_big_endian(&fact.transaction_cost),
        gas_limit: fact.gas_limit,
        propose_dag_gas_limit: fact.propose_dag_gas_limit,
        insert_non_proposable: fact.insert_non_proposable,
        in_non_finalized_cache: sidecar.0.contains_non_finalized(hash),
        in_recently_finalized_cache: sidecar.0.contains_recently_finalized(hash),
        account_found: fact.account_found,
        account_nonce: U256::from_big_endian(&fact.account_nonce),
        account_balance: U256::from_big_endian(&fact.account_balance),
    })?;
    Ok(TransactionManagerValidatedInsertPlan {
        status: queue_status_to_ffi(plan.status),
        queue_action: if !plan.should_insert_queue {
            TM_VALIDATED_INSERT_QUEUE_ACTION_NONE
        } else if plan.queue_proposable {
            TM_VALIDATED_INSERT_QUEUE_ACTION_INSERT_PROPOSABLE
        } else {
            TM_VALIDATED_INSERT_QUEUE_ACTION_INSERT_NON_PROPOSABLE
        },
        emit_transaction_added: plan.emit_transaction_added,
    })
}

/// Builds a deterministic pre-mutation plan using Rust-owned runtime queue and sidecar state.
pub fn transaction_manager_plan_validated_insert_with_runtime(
    runtime: &BridgeTransactionManagerRuntime,
    fact: TransactionManagerValidatedInsertSidecarFact,
) -> Result<TransactionManagerValidatedInsertPlan> {
    let hash = H256::from(fact.tx_hash);
    let plan = plan_validated_insert(ConsensusTransactionManagerValidatedInsertFact {
        tx_hash: hash,
        transaction_nonce: U256::from_big_endian(&fact.transaction_nonce),
        transaction_cost: U256::from_big_endian(&fact.transaction_cost),
        gas_limit: fact.gas_limit,
        propose_dag_gas_limit: fact.propose_dag_gas_limit,
        insert_non_proposable: fact.insert_non_proposable,
        in_non_finalized_cache: runtime.sidecar.contains_non_finalized(hash),
        in_recently_finalized_cache: runtime.sidecar.contains_recently_finalized(hash),
        account_found: fact.account_found,
        account_nonce: U256::from_big_endian(&fact.account_nonce),
        account_balance: U256::from_big_endian(&fact.account_balance),
    })?;
    Ok(TransactionManagerValidatedInsertPlan {
        status: queue_status_to_ffi(plan.status),
        queue_action: if !plan.should_insert_queue {
            TM_VALIDATED_INSERT_QUEUE_ACTION_NONE
        } else if plan.queue_proposable {
            TM_VALIDATED_INSERT_QUEUE_ACTION_INSERT_PROPOSABLE
        } else {
            TM_VALIDATED_INSERT_QUEUE_ACTION_INSERT_NON_PROPOSABLE
        },
        emit_transaction_added: plan.emit_transaction_added,
    })
}

/// Filters finalized transactions for one legacy C++ transaction-manager path.
///
/// Each fact already includes `recently_finalized` membership, so Rust only performs
/// storage checks for cache misses. Facts from C++ are transformed into consensus
/// facts and then back into bridge shapes without mutating live queues.
pub fn transaction_manager_filter_non_finalized(
    storage: &BridgeStorage,
    facts: Vec<TransactionManagerFinalizedFilterFact>,
) -> Result<FinalizedTransactionFilterPlan> {
    let plan: ConsensusFinalizedTransactionFilterPlan =
        plan_exclude_finalized_transactions_from_storage(
            facts
                .into_iter()
                .map(consensus_filter_fact_from_ffi_fact)
                .collect(),
            |hash| {
                storage
                    .0
                    .transaction()
                    .finalized(hash)
                    .context("TM_FILTER_FINALIZED_LOOKUP")
            },
        )?;

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

/// Verifies a transaction sequence contains no finalized hashes.
///
/// The rule mirrors `TransactionManager::verifyTransactionsNotFinalized`:
/// `recently_finalized_transactions_` short-circuits immediately and storage lookup
/// only runs when sender nonce is greater-or-equal to transaction nonce.
pub fn transaction_manager_verify_not_finalized(
    storage: &BridgeStorage,
    facts: Vec<TransactionManagerVerifyNotFinalizedFact>,
) -> Result<TransactionManagerVerifyNotFinalizedOutcome> {
    let recent_indexes = facts
        .iter()
        .filter_map(|fact| fact.in_recently_finalized_cache.then_some(fact.input_index))
        .collect::<std::collections::HashSet<_>>();
    let plan: ConsensusVerifyNotFinalizedTransactionPlan =
        plan_verify_not_finalized_transactions_from_storage(
            facts
                .into_iter()
                .map(consensus_verify_not_finalized_fact_from_ffi_fact)
                .collect(),
            |hash| {
                storage
                    .0
                    .transaction()
                    .finalized(hash)
                    .context("TM_VERIFY_FINALIZED_LOOKUP")
            },
        )?;

    let mut out = TransactionManagerVerifyNotFinalizedOutcome {
        is_finalized: false,
        input_index: 0,
        hash: [0; 32],
        source: TM_VERIFY_NOT_FINALIZED_SOURCE_NONE,
    };

    if let Some(failure) = plan.finalized {
        out.is_finalized = true;
        out.input_index = failure.input_index;
        out.hash = failure.hash.0;
        out.source = if recent_indexes.contains(&failure.input_index) {
            TM_VERIFY_NOT_FINALIZED_SOURCE_RECENT_SIDECAR
        } else {
            TM_VERIFY_NOT_FINALIZED_SOURCE_STORAGE
        };
    }

    Ok(out)
}

/// Filters finalized transactions using Rust-owned live sidecars plus storage.
pub fn transaction_manager_filter_non_finalized_with_sidecar(
    sidecar: &BridgeTransactionManagerSidecar,
    storage: &BridgeStorage,
    requests: Vec<TransactionManagerSidecarLookupRequest>,
) -> Result<FinalizedTransactionFilterPlan> {
    let facts = requests
        .into_iter()
        .map(|request| {
            let hash = H256::from(request.hash);
            ConsensusFinalizedTransactionFilterFact {
                input_index: request.input_index,
                hash,
                in_recently_finalized_cache: sidecar.0.contains_recently_finalized(hash),
            }
        })
        .collect();

    let plan: ConsensusFinalizedTransactionFilterPlan =
        plan_exclude_finalized_transactions_from_storage(facts, |hash| {
            storage
                .0
                .transaction()
                .finalized(hash)
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

/// Filters finalized transactions using Rust runtime sidecars plus storage.
pub fn transaction_manager_filter_non_finalized_with_runtime(
    runtime: &BridgeTransactionManagerRuntime,
    storage: &BridgeStorage,
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
            storage
                .0
                .transaction()
                .finalized(hash)
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

/// Verifies transaction hashes against Rust-owned recent sidecars and storage.
pub fn transaction_manager_verify_not_finalized_with_sidecar(
    sidecar: &BridgeTransactionManagerSidecar,
    storage: &BridgeStorage,
    facts: Vec<TransactionManagerVerifyNotFinalizedSidecarFact>,
) -> Result<TransactionManagerVerifyNotFinalizedOutcome> {
    for fact in facts {
        let hash = H256::from(fact.hash);
        ensure!(
            !hash.is_zero(),
            "finalized verification transaction hash cannot be zero"
        );
        if sidecar.0.contains_recently_finalized(hash) {
            return Ok(TransactionManagerVerifyNotFinalizedOutcome {
                is_finalized: true,
                input_index: fact.input_index,
                hash: hash.0,
                source: TM_VERIFY_NOT_FINALIZED_SOURCE_RECENT_SIDECAR,
            });
        }

        if U256::from_big_endian(&fact.sender_account_nonce)
            >= U256::from_big_endian(&fact.transaction_nonce)
            && storage
                .0
                .transaction()
                .finalized(hash)
                .context("TM_VERIFY_FINALIZED_LOOKUP")?
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

const TM_STORED_TX_SOURCE_MISSING: u8 = 0;
const TM_STORED_TX_SOURCE_PENDING: u8 = 1;
const TM_STORED_TX_SOURCE_FINALIZED_REGULAR: u8 = 2;
const TM_STORED_TX_SOURCE_FINALIZED_SYSTEM: u8 = 3;
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

/// Bridge-owned Rust TransactionManager sidecar wrapper.
pub struct BridgeTransactionManagerSidecar(pub TransactionManagerSidecar);

/// Resolves transaction hashes through TransactionManager storage rules.
///
/// Inputs are ordered requests from C++ after live transaction caches miss.
/// Outputs preserve request order, echo `input_index` and `hash`, classify the
/// storage source, and carry canonical transaction RLP bytes for C++ object
/// materialization. Missing hashes are returned as `source = 0` instead of
/// errors. Storage backend failures, malformed transaction-location RLP, and
/// malformed period data return `anyhow::Error` with stable context labels.
pub fn transaction_manager_load_stored_transactions(
    storage: &BridgeStorage,
    requests: Vec<TransactionManagerStoredTransactionRequest>,
) -> Result<Vec<TransactionManagerStoredTransactionLookup>> {
    let mut out = Vec::with_capacity(requests.len());
    let transaction = storage.0.transaction();

    for request in requests {
        let hash = H256::from(request.hash);
        let (tx_rlp, source) = if let Some(tx_rlp) = transaction
            .rlp(hash)
            .context("TM_TRANSACTION_RLP_PENDING_LOOKUP")?
        {
            (Some(tx_rlp), TM_STORED_TX_SOURCE_PENDING)
        } else if let Some(location_rlp) = transaction
            .location_rlp(hash)
            .context("TM_TRANSACTION_RLP_LOCATION_LOOKUP")?
        {
            let location = rlp::Rlp::new(&location_rlp);
            let period = location
                .val_at::<u64>(0)
                .context("TM_TRANSACTION_RLP_LOCATION_PERIOD")?;
            let position = location
                .val_at::<u32>(1)
                .context("TM_TRANSACTION_RLP_LOCATION_POSITION")?;
            let is_system = location
                .item_count()
                .context("TM_TRANSACTION_RLP_LOCATION_SHAPE")?
                == 3
                && location
                    .val_at::<bool>(2)
                    .context("TM_TRANSACTION_RLP_LOCATION_SYSTEM_FLAG")?;
            let tx_rlp = if is_system {
                transaction
                    .system_rlp(hash)
                    .context("TM_TRANSACTION_RLP_SYSTEM_LOOKUP")?
                    .map(|tx_rlp| (tx_rlp, TM_STORED_TX_SOURCE_FINALIZED_SYSTEM))
            } else {
                transaction
                    .by_period_position_rlp(period, position)
                    .context("TM_TRANSACTION_RLP_FINALIZED_LOOKUP")?
                    .map(|tx_rlp| (tx_rlp, TM_STORED_TX_SOURCE_FINALIZED_REGULAR))
            };

            match tx_rlp {
                Some((tx_rlp, source)) => (Some(tx_rlp), source),
                None => (None, TM_STORED_TX_SOURCE_MISSING),
            }
        } else {
            (None, TM_STORED_TX_SOURCE_MISSING)
        };

        out.push(TransactionManagerStoredTransactionLookup {
            input_index: request.input_index,
            hash: hash.0,
            found: tx_rlp.is_some(),
            source,
            old_finalized: false,
            tx_rlp: tx_rlp.unwrap_or_default(),
        });
    }

    Ok(out)
}

/// Resolves storage-backed proposal transactions and filters finalized hits
/// against Rust FinalChain account state at the proposal period.
///
/// The generic storage lookup contract remains byte-oriented. This proposal
/// path additionally verifies the stored RLP hash, inspects the legacy
/// transaction sender and nonce in Rust, and returns old finalized
/// transactions as data misses with `old_finalized = true` so C++ only
/// materializes accepted payloads.
pub fn transaction_manager_load_proposal_transactions_with_final_chain(
    storage: &BridgeStorage,
    final_chain: &BridgeFinalChain,
    proposal_period: u64,
    requests: Vec<TransactionManagerStoredTransactionRequest>,
) -> Result<Vec<TransactionManagerStoredTransactionLookup>> {
    let lookups = transaction_manager_load_stored_transactions(storage, requests)?;
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

/// Verifies transaction hashes against Rust runtime recent sidecars and storage.
pub fn transaction_manager_verify_not_finalized_with_runtime(
    runtime: &BridgeTransactionManagerRuntime,
    storage: &BridgeStorage,
    facts: Vec<TransactionManagerVerifyNotFinalizedSidecarFact>,
) -> Result<TransactionManagerVerifyNotFinalizedOutcome> {
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
            && storage
                .0
                .transaction()
                .finalized(hash)
                .context("TM_VERIFY_FINALIZED_LOOKUP")?
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

/// Verifies transaction hashes against Rust runtime sidecars with sender nonce
/// sourced from latest FinalChain state when storage lookup is safe.
pub fn transaction_manager_verify_not_finalized_with_runtime_and_final_chain(
    runtime: &BridgeTransactionManagerRuntime,
    storage: &BridgeStorage,
    final_chain: &BridgeFinalChain,
    facts: Vec<TransactionManagerVerifyNotFinalizedRuntimeFact>,
) -> Result<TransactionManagerVerifyNotFinalizedOutcome> {
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
        let (_, sender_nonce, _) = final_chain_account_lookup(final_chain, &fact.sender)?;
        if sender_nonce >= U256::from_big_endian(&fact.transaction_nonce)
            && storage
                .0
                .transaction()
                .finalized(hash)
                .context("TM_VERIFY_FINALIZED_LOOKUP")?
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

/// Returns all payloads currently persisted for non-finalized transaction recovery.
///
/// The returned list preserves storage iteration order, carries the DB key hash
/// for invariant validation by C++, and flags any payloads that are stale by
/// checking finalized-index membership in Rust. Stale finalized rows are removed
/// from non-finalized storage in one Rust storage batch before returning.
pub fn transaction_manager_load_nonfinalized_recovery(
    storage: &BridgeStorage,
) -> Result<Vec<TransactionManagerRecoveryEntry>> {
    let transaction = storage.0.transaction();
    let non_finalized = transaction.all_nonfinalized_with_hash()?;
    let mut out = Vec::with_capacity(non_finalized.len());
    let mut stale_hashes = Vec::new();

    for (hash, tx_rlp) in non_finalized {
        let finalized = transaction
            .finalized(hash)
            .context("TM_NONFINALIZED_RECOVERY_FINALIZED_LOOKUP")?;
        if finalized {
            stale_hashes.push(hash);
        }

        out.push(TransactionManagerRecoveryEntry {
            hash: hash.0,
            finalized,
            tx_rlp,
        });
    }

    if !stale_hashes.is_empty() {
        let mut batch = storage.0.create_write_batch();
        for hash in stale_hashes {
            storage
                .0
                .batch_delete_raw(
                    &mut batch,
                    rustaxa_storage::Column::Transactions,
                    hash.as_bytes(),
                )
                .context("TM_NONFINALIZED_RECOVERY_STALE_DELETE")?;
        }
        storage
            .0
            .commit_write_batch_with_sync(batch, false)
            .context("TM_NONFINALIZED_RECOVERY_STALE_COMMIT")?;
    }

    Ok(out)
}

/// Returns Rust-validated sidecar inputs for non-finalized transaction recovery.
///
/// Rust owns storage iteration, stale-finalized cleanup, canonical legacy RLP
/// inspection, key-hash validation, and sender-presence validation before C++
/// mutates live runtime sidecars. C++ only applies the returned inputs under the
/// transaction mutex, so malformed survivor storage never reaches live state.
pub fn transaction_manager_load_nonfinalized_recovery_inputs(
    storage: &BridgeStorage,
) -> Result<Vec<TransactionManagerSidecarRecoveryInsertInput>> {
    let entries = transaction_manager_load_nonfinalized_recovery(storage)?;
    let mut recovered = Vec::with_capacity(entries.len());

    for entry in entries {
        if entry.finalized {
            continue;
        }

        let inspection = legacy_transaction_inspection_from_bytes(&entry.tx_rlp, 0)
            .context("TM_NONFINALIZED_RECOVERY_ENVELOPE_INSPECT")?;
        ensure!(
            inspection.hash == entry.hash,
            "TM_NONFINALIZED_RECOVERY_HASH_MISMATCH"
        );
        ensure!(
            inspection.sender_found,
            "TM_NONFINALIZED_RECOVERY_SENDER_MISSING"
        );

        recovered.push(TransactionManagerSidecarRecoveryInsertInput {
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
    storage: &BridgeStorage,
) -> Result<()> {
    let entries = transaction_manager_load_nonfinalized_recovery_inputs(storage)?;
    runtime
        .transaction_manager_runtime_insert_recovery_entries(entries)
        .map(|_| ())
}

/// Creates a Rust-owned TransactionManager sidecar seeded from persisted manager state.
pub fn create_transaction_manager_sidecar(
    initial_transaction_count: u64,
) -> Box<BridgeTransactionManagerSidecar> {
    Box::new(BridgeTransactionManagerSidecar(
        TransactionManagerSidecar::new(initial_transaction_count),
    ))
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
    let gas_estimation_cache_size = config.max_size / 10;
    let gas_estimation_cache_delete_step = config.max_size / 100;
    Box::new(BridgeTransactionManagerRuntime {
        sidecar: TransactionManagerSidecar::new_with_gas_estimation_cache(
            initial_transaction_count,
            gas_estimation_cache_size,
            gas_estimation_cache_delete_step,
        ),
        queue: TransactionQueue::new(config.max_size as u64),
        last_drop_observed: None,
        transaction_pack_session: None,
    })
}

impl BridgeTransactionManagerRuntime {
    /// Begins one runtime-owned transaction packing session.
    ///
    /// The runtime snapshots ordered queue payloads up to the planner candidate
    /// cap. C++ then asks Rust for one estimable candidate at a time and never
    /// owns the deterministic candidate scan or accepted ordering.
    pub fn transaction_manager_runtime_pack_begin(
        &mut self,
        weight_limit: u64,
        min_transaction_gas: u64,
        proposal_period: u64,
        estimate_gas_limit: u64,
        last_block_number: u64,
    ) -> Result<()> {
        ensure!(
            self.transaction_pack_session.is_none(),
            "TM_RUNTIME_PACK_SESSION_ALREADY_ACTIVE"
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

    /// Returns the number of retained gas-estimation cache entries.
    pub fn transaction_manager_runtime_gas_estimation_cache_size(&self) -> usize {
        self.sidecar.gas_estimation_cache_len()
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

    /// Returns Rust's known-transaction decision using runtime queue and sidecar state.
    pub fn transaction_manager_runtime_is_transaction_known(
        &self,
        fact: TransactionManagerSidecarKnownFact,
    ) -> Result<bool> {
        self.sidecar
            .is_transaction_known(TransactionManagerKnownFact {
                hash: H256::from(fact.hash),
                queue_known: self.queue.is_transaction_known(H256::from(fact.hash)),
            })
            .context("TM_RUNTIME_IS_TRANSACTION_KNOWN")
    }

    /// Returns bounded, source-ordered runtime payload views from queue, sidecars, and storage.
    pub fn transaction_manager_runtime_lookup_transaction_views(
        &self,
        storage: &BridgeStorage,
        requests: Vec<TransactionManagerTransactionViewRequest>,
        max_count: u64,
    ) -> Result<TransactionManagerTransactionViewPlan> {
        transaction_manager_runtime_lookup_transaction_views_inner(
            self,
            requests,
            max_count,
            |stored_requests| {
                transaction_manager_load_stored_transactions(storage, stored_requests)
            },
        )
    }

    /// Returns bounded, source-ordered runtime payload views including proposal-period filtering.
    pub fn transaction_manager_runtime_lookup_proposal_transaction_views(
        &self,
        storage: &BridgeStorage,
        final_chain: &BridgeFinalChain,
        proposal_period: u64,
        requests: Vec<TransactionManagerTransactionViewRequest>,
        max_count: u64,
    ) -> Result<TransactionManagerTransactionViewPlan> {
        transaction_manager_runtime_lookup_transaction_views_inner(
            self,
            requests,
            max_count,
            |stored_requests| {
                transaction_manager_load_proposal_transactions_with_final_chain(
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
    pub fn transaction_manager_runtime_insert_non_finalized(
        &mut self,
        input: TransactionManagerSidecarInsertInput,
    ) -> Result<()> {
        self.sidecar
            .insert_non_finalized(H256::from(input.hash), input.trx_rlp)
            .context("TM_RUNTIME_INSERT_NON_FINALIZED")
    }

    /// True when hash exists in non-finalized sidecar state.
    pub fn transaction_manager_runtime_contains_non_finalized(&self, hash: &[u8; 32]) -> bool {
        self.sidecar.contains_non_finalized(H256::from(*hash))
    }

    /// True when hash exists in recently-finalized sidecar state.
    pub fn transaction_manager_runtime_contains_recently_finalized(&self, hash: &[u8; 32]) -> bool {
        self.sidecar.contains_recently_finalized(H256::from(*hash))
    }

    /// Returns ordered sidecar payload lookups for C++ materialization.
    pub fn transaction_manager_runtime_lookup_ordered_payloads(
        &self,
        requests: Vec<TransactionManagerSidecarLookupRequest>,
    ) -> Result<TransactionManagerSidecarLookupPlan> {
        let lookups = self
            .sidecar
            .lookup_payloads_ordered(
                requests
                    .iter()
                    .map(|request| (request.input_index, H256::from(request.hash)))
                    .collect(),
            )
            .context("TM_RUNTIME_LOOKUP_ORDERED")?;
        Ok(TransactionManagerSidecarLookupPlan {
            lookups: lookups
                .into_iter()
                .map(|lookup| TransactionManagerSidecarLookup {
                    input_index: lookup.input_index,
                    hash: lookup.hash.0,
                    found: lookup.found,
                    source: lookup.source,
                    trx_rlp: lookup.trx_rlp,
                })
                .collect(),
        })
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
        let mut removed = 0u64;
        for request in requests {
            let hash = H256::from(request.hash);
            ensure!(
                !hash.is_zero(),
                "runtime sidecar removal hash cannot be zero"
            );
            if self.sidecar.remove_non_finalized(hash) {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Moves finalized hashes from non-finalized to recently-finalized sidecar state.
    pub fn transaction_manager_runtime_apply_finalized_transition(
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

    /// Evicts stale recently-finalized entries for one computed stale period.
    pub fn transaction_manager_runtime_evict_stale_recently_finalized(
        &mut self,
        stale_period: u64,
    ) -> u64 {
        self.sidecar
            .evict_recently_finalized_stale_period(stale_period) as u64
    }

    /// Inserts recovery payloads while skipping stale finalized entries.
    pub fn transaction_manager_runtime_insert_recovery_entries(
        &mut self,
        entries: Vec<TransactionManagerSidecarRecoveryInsertInput>,
    ) -> Result<u64> {
        Ok(self
            .sidecar
            .insert_recovery_entries(
                entries
                    .into_iter()
                    .map(|entry| ConsensusTransactionManagerSidecarRecoveryEntry {
                        hash: H256::from(entry.hash),
                        finalized: entry.finalized,
                        trx_rlp: entry.trx_rlp,
                    })
                    .collect(),
            )
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

    /// Executes validated-insert planning and queue mutation through a single runtime boundary.
    pub fn transaction_manager_runtime_insert_validated_transaction(
        &mut self,
        fact: TransactionManagerValidatedInsertSidecarFact,
        input: TransactionQueueInsertInput,
    ) -> Result<TransactionManagerRuntimeValidatedInsertOutcome> {
        let outcome =
            self.transaction_manager_runtime_execute_transaction_admission(fact, input, false, 0)?;
        Ok(TransactionManagerRuntimeValidatedInsertOutcome {
            status: outcome.transaction_status,
            emit_transaction_added: outcome.emit_transaction_added,
            inserted_hash_found: outcome.inserted_hash_found,
            inserted_hash: outcome.inserted_hash,
            demoted_hashes: outcome.demoted_hashes,
            overflow_removed_hashes: outcome.overflow_removed_hashes,
        })
    }

    /// Executes validated-insert planning and queue mutation with account facts
    /// sourced from latest FinalChain state.
    pub fn transaction_manager_runtime_insert_validated_transaction_with_final_chain(
        &mut self,
        final_chain: &BridgeFinalChain,
        fact: TransactionManagerValidatedInsertRuntimeFact,
        input: TransactionQueueInsertInput,
    ) -> Result<TransactionManagerRuntimeValidatedInsertOutcome> {
        let outcome = self
            .transaction_manager_runtime_execute_transaction_admission_with_final_chain(
                final_chain,
                fact,
                input,
            )?;
        Ok(TransactionManagerRuntimeValidatedInsertOutcome {
            status: outcome.transaction_status,
            emit_transaction_added: outcome.emit_transaction_added,
            inserted_hash_found: outcome.inserted_hash_found,
            inserted_hash: outcome.inserted_hash,
            demoted_hashes: outcome.demoted_hashes,
            overflow_removed_hashes: outcome.overflow_removed_hashes,
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

    /// Finishes public `insertTransaction` status selection after a caller supplies finalized-location facts.
    ///
    /// This is intentionally read-only and remains for lower-level parity tests
    /// and callers that have already completed fact sourcing. Rust-mode
    /// TransactionManager production paths use the FinalChain- or storage-backed
    /// admission helpers so no C++ storage completion is required after mutation.
    pub fn transaction_manager_runtime_finish_insert_transaction(
        &self,
        fact: TransactionManagerInsertTransactionFact,
    ) -> Result<TransactionManagerInsertTransactionOutcome> {
        let _ = self;
        transaction_manager_insert_transaction(fact)
    }

    /// Executes TransactionManager admission and returns both public and queue statuses.
    ///
    /// Inputs:
    /// - `fact` contains caller-supplied account facts and transaction metadata.
    /// - `input` contains canonical queue metadata and RLP payload bytes.
    /// - finalized-period fields are C++ storage facts used only when admission
    ///   resolves to a known/finalized transaction.
    ///
    /// Behavior and invariants:
    /// - rejects hash/nonce/gas mismatches before mutation.
    /// - Rust folds sidecar membership into validated-admission planning.
    /// - Rust mutates the queue only after planning says queue insertion is required.
    /// - Rust maps the resulting queue status into the public `insertTransaction`
    ///   status so C++ does not infer admission intent from queue status locally.
    pub fn transaction_manager_runtime_execute_transaction_admission(
        &mut self,
        fact: TransactionManagerValidatedInsertSidecarFact,
        mut input: TransactionQueueInsertInput,
        has_finalized_period: bool,
        finalized_period: u64,
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
            account_found: fact.account_found,
            account_nonce: U256::from_big_endian(&fact.account_nonce),
            account_balance: U256::from_big_endian(&fact.account_balance),
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
                has_finalized_period,
                finalized_period,
            })?;

        Ok(TransactionManagerRuntimeAdmissionOutcome {
            insert_status: insert_outcome.status,
            transaction_status: queue_outcome.status,
            requires_finalized_lookup: queue_outcome.status
                == TransactionQueueInsertStatus::Known as u8
                && !insert_outcome.finalized_period_known,
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

    /// Executes admission and returns a typed command report for C++ side effects.
    pub fn transaction_manager_runtime_execute_transaction_admission_command_report(
        &mut self,
        fact: TransactionManagerValidatedInsertSidecarFact,
        input: TransactionQueueInsertInput,
        has_finalized_period: bool,
        finalized_period: u64,
    ) -> Result<TransactionManagerAdmissionCommandReport> {
        let outcome = self.transaction_manager_runtime_execute_transaction_admission(
            fact,
            input,
            has_finalized_period,
            finalized_period,
        )?;
        Ok(admission_command_report(&outcome))
    }

    /// Executes TransactionManager admission using storage for finalized-location completion.
    ///
    /// Storage-backed finalized lookup is performed before queue mutation to avoid
    /// C++ fallback lookup overhead and keep finalized/fallback decisioning inside
    /// Rust.
    pub fn transaction_manager_runtime_execute_transaction_admission_with_storage(
        &mut self,
        storage: &BridgeStorage,
        fact: TransactionManagerValidatedInsertSidecarFact,
        input: TransactionQueueInsertInput,
    ) -> Result<TransactionManagerRuntimeAdmissionOutcome> {
        let finalized_period = storage_transaction_period_lookup(storage, &fact.tx_hash)?;
        let (has_finalized_period, finalized_period) = match finalized_period {
            Some(period) => (true, period),
            None => (false, 0),
        };
        let mut outcome = self.transaction_manager_runtime_execute_transaction_admission(
            fact,
            input,
            has_finalized_period,
            finalized_period,
        )?;
        outcome.requires_finalized_lookup = false;
        Ok(outcome)
    }

    /// Executes storage-backed admission and returns a typed command report.
    pub fn transaction_manager_runtime_execute_transaction_admission_with_storage_command_report(
        &mut self,
        storage: &BridgeStorage,
        fact: TransactionManagerValidatedInsertSidecarFact,
        input: TransactionQueueInsertInput,
    ) -> Result<TransactionManagerAdmissionCommandReport> {
        let outcome = self.transaction_manager_runtime_execute_transaction_admission_with_storage(
            storage, fact, input,
        )?;
        Ok(admission_command_report(&outcome))
    }

    /// Executes TransactionManager admission using account/finalization facts
    /// sourced from FinalChain.
    ///
    /// Account and finalized-location reads complete before queue mutation so
    /// lookup failures cannot partially admit a transaction into Rust runtime
    /// state.
    pub fn transaction_manager_runtime_execute_transaction_admission_with_final_chain(
        &mut self,
        final_chain: &BridgeFinalChain,
        fact: TransactionManagerValidatedInsertRuntimeFact,
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
        let (account_found, account_nonce, account_balance) =
            final_chain_account_lookup(final_chain, &fact.sender)?;
        let finalized_period = final_chain_transaction_period_lookup(final_chain, &fact.tx_hash)?;
        let plan = plan_validated_insert(ConsensusTransactionManagerValidatedInsertFact {
            tx_hash: hash,
            transaction_nonce: U256::from_big_endian(&fact.transaction_nonce),
            transaction_cost: U256::from_big_endian(&fact.transaction_cost),
            gas_limit: fact.gas_limit,
            propose_dag_gas_limit: fact.propose_dag_gas_limit,
            insert_non_proposable: fact.insert_non_proposable,
            in_non_finalized_cache: self.sidecar.contains_non_finalized(hash),
            in_recently_finalized_cache: self.sidecar.contains_recently_finalized(hash),
            account_found,
            account_nonce,
            account_balance,
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
                has_finalized_period: finalized_period.is_some(),
                finalized_period: finalized_period.unwrap_or_default(),
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

    /// Executes FinalChain-backed admission and returns a typed command report.
    pub fn transaction_manager_runtime_execute_transaction_admission_with_final_chain_command_report(
        &mut self,
        final_chain: &BridgeFinalChain,
        fact: TransactionManagerValidatedInsertRuntimeFact,
        input: TransactionQueueInsertInput,
    ) -> Result<TransactionManagerAdmissionCommandReport> {
        let outcome = self
            .transaction_manager_runtime_execute_transaction_admission_with_final_chain(
                final_chain,
                fact,
                input,
            )?;
        ensure!(
            !outcome.requires_finalized_lookup,
            "TM_RUNTIME_FINAL_CHAIN_ADMISSION_LOOKUP_INCOMPLETE"
        );
        Ok(admission_command_report(&outcome))
    }

    /// Executes public insert precheck, verification, and FinalChain-backed admission.
    pub fn transaction_manager_runtime_execute_public_transaction_admission_with_final_chain_command_report(
        &mut self,
        final_chain: &BridgeFinalChain,
        verify_fact: TransactionManagerVerifyTransactionFact,
        admission_fact: TransactionManagerValidatedInsertRuntimeFact,
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
            .transaction_manager_runtime_execute_transaction_admission_with_final_chain_command_report(
                final_chain,
                admission_fact,
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

    /// Removes one queued transaction by hash.
    pub fn transaction_manager_runtime_queue_erase(&mut self, hash: &[u8; 32]) -> bool {
        self.queue.erase(H256::from(*hash))
    }

    /// Returns one queued transaction payload by hash.
    pub fn transaction_manager_runtime_queue_get_transaction(
        &self,
        hash: &[u8; 32],
    ) -> TransactionQueueStoredTransaction {
        runtime_queue_stored_transaction_from_entry(self.queue.transaction(H256::from(*hash)))
    }

    /// Returns proposer-ordered transaction payloads.
    pub fn transaction_manager_runtime_queue_ordered_transactions(
        &self,
        count: u64,
    ) -> Vec<TransactionQueueStoredTransaction> {
        self.queue
            .ordered_transactions(count)
            .into_iter()
            .map(Some)
            .map(runtime_queue_stored_transaction_from_entry)
            .collect()
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
    pub fn transaction_manager_runtime_queue_contains(&self, hash: &[u8; 32]) -> bool {
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

    /// Returns proposer accounts for fact-driven parity tests.
    pub fn transaction_manager_runtime_queue_proposable_accounts(
        &self,
    ) -> Vec<TransactionQueueAddress> {
        self.queue
            .proposable_accounts()
            .into_iter()
            .map(|address| TransactionQueueAddress { address: address.0 })
            .collect()
    }

    /// Removes queued transactions for caller-supplied account nonce facts.
    ///
    /// This API is retained for parity tests and compatibility scaffolding.
    /// Rust-enabled production queue cleanup should call
    /// `transaction_manager_runtime_queue_cleanup_with_final_chain` so account
    /// fact sourcing stays inside Rust.
    pub fn transaction_manager_runtime_queue_purge_accounts_plan(
        &mut self,
        facts: Vec<BridgeTransactionQueueAccountNonceFact>,
    ) -> TransactionQueuePurgePlan {
        let consensus_facts = runtime_queue_account_nonce_facts_from_bridge(facts);
        runtime_queue_purge_plan_from_consensus(self.queue.purge_accounts_plan(&consensus_facts))
    }

    /// Applies Rust-owned queue cleanup for finalized block height and caller-supplied account facts.
    ///
    /// This fact-driven API is retained for parity tests and compatibility
    /// scaffolding. Rust-enabled production queue cleanup should call
    /// `transaction_manager_runtime_queue_cleanup_with_final_chain` so account
    /// fact sourcing stays inside Rust. Rust still owns all queue mutation and
    /// returns explicit removed hash groups for C++ logging or future
    /// side-effect execution.
    pub fn transaction_manager_runtime_queue_cleanup(
        &mut self,
        apply_block_finalized: bool,
        block_number: u64,
        facts: Vec<BridgeTransactionQueueAccountNonceFact>,
    ) -> TransactionManagerRuntimeQueueCleanupPlan {
        let non_proposable_expired = if apply_block_finalized {
            self.queue.block_finalized_plan(block_number)
        } else {
            TransactionQueuePurgeOutcome::default()
        };
        let consensus_facts = runtime_queue_account_nonce_facts_from_bridge(facts);
        let finalized_account_purged = self.queue.purge_accounts_plan(&consensus_facts);
        TransactionManagerRuntimeQueueCleanupPlan {
            non_proposable_expired: runtime_queue_purge_plan_from_consensus(non_proposable_expired),
            finalized_account_purged: runtime_queue_purge_plan_from_consensus(
                finalized_account_purged,
            ),
        }
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

    /// Marks one hash in the Rust-owned known-admission cache.
    pub fn transaction_manager_runtime_queue_mark_transaction_known(
        &mut self,
        hash: &[u8; 32],
    ) -> bool {
        self.queue.mark_transaction_known(H256::from(*hash))
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

impl BridgeTransactionManagerSidecar {
    /// Returns the authoritative Rust-mode manager transaction count.
    pub fn transaction_manager_sidecar_transaction_count(&self) -> u64 {
        self.0.transaction_count()
    }

    /// Returns Rust's known-transaction admission decision from queue and sidecar facts.
    pub fn transaction_manager_sidecar_is_transaction_known(
        &self,
        fact: TransactionManagerSidecarKnownFact,
    ) -> Result<bool> {
        self.0
            .is_transaction_known(TransactionManagerKnownFact {
                hash: H256::from(fact.hash),
                queue_known: fact.queue_known,
            })
            .context("TM_SIDECAR_IS_TRANSACTION_KNOWN")
    }

    /// Inserts or updates one live non-finalized sidecar payload.
    pub fn transaction_manager_sidecar_insert_non_finalized(
        &mut self,
        input: TransactionManagerSidecarInsertInput,
    ) -> Result<()> {
        self.0
            .insert_non_finalized(H256::from(input.hash), input.trx_rlp)
            .context("TM_SIDECAR_INSERT_NON_FINALIZED")
    }

    /// True when hash exists in non-finalized sidecar state.
    pub fn transaction_manager_sidecar_contains_non_finalized(&self, hash: &[u8; 32]) -> bool {
        self.0.contains_non_finalized(H256::from(*hash))
    }

    /// True when hash exists in recently-finalized sidecar state.
    pub fn transaction_manager_sidecar_contains_recently_finalized(&self, hash: &[u8; 32]) -> bool {
        self.0.contains_recently_finalized(H256::from(*hash))
    }

    /// Returns ordered payload lookups for C++ transaction materialization.
    pub fn transaction_manager_sidecar_lookup_ordered_payloads(
        &self,
        requests: Vec<TransactionManagerSidecarLookupRequest>,
    ) -> Result<TransactionManagerSidecarLookupPlan> {
        let lookups = self
            .0
            .lookup_payloads_ordered(
                requests
                    .iter()
                    .map(|request| (request.input_index, H256::from(request.hash)))
                    .collect(),
            )
            .context("TM_SIDECAR_LOOKUP_ORDERED")?;
        Ok(TransactionManagerSidecarLookupPlan {
            lookups: lookups
                .into_iter()
                .map(|lookup| TransactionManagerSidecarLookup {
                    input_index: lookup.input_index,
                    hash: lookup.hash.0,
                    found: lookup.found,
                    source: lookup.source,
                    trx_rlp: lookup.trx_rlp,
                })
                .collect(),
        })
    }

    /// Returns current non-finalized sidecar size.
    pub fn transaction_manager_sidecar_non_finalized_size(&self) -> usize {
        self.0.non_finalized_size()
    }

    /// Removes requested non-finalized sidecar payloads and returns the removal count.
    pub fn transaction_manager_sidecar_remove_non_finalized(
        &mut self,
        requests: Vec<TransactionManagerSidecarLookupRequest>,
    ) -> Result<u64> {
        let mut removed = 0u64;
        for request in requests {
            let hash = H256::from(request.hash);
            ensure!(!hash.is_zero(), "sidecar removal hash cannot be zero");
            if self.0.remove_non_finalized(hash) {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Moves finalized hashes from non-finalized to recently-finalized sidecar state.
    pub fn transaction_manager_sidecar_apply_finalized_transition(
        &mut self,
        transition: TransactionManagerSidecarTransitionInput,
    ) -> Result<()> {
        self.0
            .apply_finalized_transition(
                transition.period,
                transition
                    .hashes
                    .into_iter()
                    .map(|hash| H256::from(hash.hash))
                    .collect::<Vec<_>>(),
            )
            .context("TM_SIDECAR_FINALIZED_TRANSITION")
    }

    /// Evicts stale recently-finalized entries for one computed stale period.
    pub fn transaction_manager_sidecar_evict_stale_recently_finalized(
        &mut self,
        stale_period: u64,
    ) -> u64 {
        self.0.evict_recently_finalized_stale_period(stale_period) as u64
    }

    /// Inserts recovery payloads while skipping stale finalized entries.
    pub fn transaction_manager_sidecar_insert_recovery_entries(
        &mut self,
        entries: Vec<TransactionManagerSidecarRecoveryInsertInput>,
    ) -> Result<u64> {
        Ok(self
            .0
            .insert_recovery_entries(
                entries
                    .into_iter()
                    .map(|entry| ConsensusTransactionManagerSidecarRecoveryEntry {
                        hash: H256::from(entry.hash),
                        finalized: entry.finalized,
                        trx_rlp: entry.trx_rlp,
                    })
                    .collect(),
            )
            .context("TM_SIDECAR_RECOVERY_INSERT")? as u64)
    }
}

fn consensus_fact_from_ffi_fact(fact: DagTransactionSaveFact) -> ConsensusDagTransactionSaveFact {
    ConsensusDagTransactionSaveFact {
        input_index: fact.input_index,
        hash: H256::from(fact.hash),
        trx_rlp: fact.trx_rlp,
        transaction_nonce: U256::from_big_endian(&fact.transaction_nonce),
        sender_account_nonce: U256::from_big_endian(&fact.sender_account_nonce),
        in_non_finalized_cache: fact.in_non_finalized_cache,
        in_recently_finalized_cache: fact.in_recently_finalized_cache,
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

fn consensus_validated_insert_fact_from_ffi_fact(
    fact: TransactionManagerValidatedInsertFact,
) -> ConsensusTransactionManagerValidatedInsertFact {
    ConsensusTransactionManagerValidatedInsertFact {
        tx_hash: H256::from(fact.tx_hash),
        transaction_nonce: U256::from_big_endian(&fact.transaction_nonce),
        transaction_cost: U256::from_big_endian(&fact.transaction_cost),
        gas_limit: fact.gas_limit,
        propose_dag_gas_limit: fact.propose_dag_gas_limit,
        insert_non_proposable: fact.insert_non_proposable,
        in_non_finalized_cache: fact.in_non_finalized_cache,
        in_recently_finalized_cache: fact.in_recently_finalized_cache,
        account_found: fact.account_found,
        account_nonce: U256::from_big_endian(&fact.account_nonce),
        account_balance: U256::from_big_endian(&fact.account_balance),
    }
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

fn consensus_finalized_status_fact_from_ffi_fact(
    fact: FinalizedTransactionStatusFact,
) -> ConsensusFinalizedTransactionStatusFact {
    ConsensusFinalizedTransactionStatusFact {
        input_index: fact.input_index,
        hash: H256::from(fact.hash),
        in_non_finalized_cache: fact.in_non_finalized_cache,
    }
}

fn consensus_filter_fact_from_ffi_fact(
    fact: TransactionManagerFinalizedFilterFact,
) -> ConsensusFinalizedTransactionFilterFact {
    ConsensusFinalizedTransactionFilterFact {
        input_index: fact.input_index,
        hash: H256::from(fact.hash),
        in_recently_finalized_cache: fact.in_recently_finalized_cache,
    }
}

fn consensus_verify_not_finalized_fact_from_ffi_fact(
    fact: TransactionManagerVerifyNotFinalizedFact,
) -> ConsensusVerifyNotFinalizedTransactionFact {
    ConsensusVerifyNotFinalizedTransactionFact {
        input_index: fact.input_index,
        hash: H256::from(fact.hash),
        transaction_nonce: U256::from_big_endian(&fact.transaction_nonce),
        sender_account_nonce: U256::from_big_endian(&fact.sender_account_nonce),
        in_recently_finalized_cache: fact.in_recently_finalized_cache,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            .save_period_data(
                1,
                period_data_rlp_with_pbft(&pbft_block, std::slice::from_ref(&transaction_rlp)),
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

        let before = transaction_manager_load_proposal_transactions_with_final_chain(
            &storage,
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

        let after = transaction_manager_load_proposal_transactions_with_final_chain(
            &storage,
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

        let mut runtime =
            create_transaction_manager_runtime(11, TransactionQueueConfig { max_size: 32 });
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
            .transaction_manager_runtime_insert_non_finalized(
                TransactionManagerSidecarInsertInput {
                    hash: [3u8; 32],
                    trx_rlp: vec![0x33],
                },
            )
            .expect("recently-finalized sidecar seed should insert");
        runtime
            .transaction_manager_runtime_apply_finalized_transition(
                TransactionManagerSidecarTransitionInput {
                    period: 10,
                    hashes: vec![crate::ffi::rustaxa_ffi::TransactionManagerSidecarHash {
                        hash: [3u8; 32],
                    }],
                },
            )
            .expect("sidecar finalized transition should move source");

        storage
            .save_transaction(&[4u8; 32], vec![0x44])
            .expect("storage pending payload should persist");

        storage
            .save_transaction_location(&[5u8; 32], 99, 0, false)
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
            .save_period_data(99, period_data.out().as_ref().to_vec())
            .expect("finalized period data should persist");

        let plan = runtime
            .transaction_manager_runtime_lookup_transaction_views(
                &storage,
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
            .save_transaction_location(&transaction_hash, 1, 0, false)
            .expect("proposal storage location should persist");
        storage
            .save_period_data(
                1,
                period_data_rlp_with_pbft(&pbft_block, std::slice::from_ref(&transaction_rlp)),
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

        let runtime =
            create_transaction_manager_runtime(11, TransactionQueueConfig { max_size: 32 });
        let plan = runtime
            .transaction_manager_runtime_lookup_proposal_transaction_views(
                &storage,
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

    fn dag_tx_fact(
        input_index: u64,
        hash: u8,
        tx_nonce: u64,
        sender_nonce: u64,
        in_non_finalized_cache: bool,
        in_recently_finalized_cache: bool,
        rlp: u8,
    ) -> DagTransactionSaveFact {
        DagTransactionSaveFact {
            input_index,
            hash: [hash; 32],
            trx_rlp: vec![rlp],
            transaction_nonce: u256_bytes(tx_nonce),
            sender_account_nonce: u256_bytes(sender_nonce),
            in_non_finalized_cache,
            in_recently_finalized_cache,
        }
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

    fn dag_tx_runtime_fact(
        input_index: u64,
        hash: u8,
        tx_nonce: u64,
        sender: [u8; 20],
        rlp: u8,
    ) -> DagTransactionSaveRuntimeFact {
        DagTransactionSaveRuntimeFact {
            input_index,
            hash: [hash; 32],
            trx_rlp: vec![rlp],
            transaction_nonce: u256_bytes(tx_nonce),
            sender,
        }
    }

    fn u256_bytes(value: u64) -> [u8; 32] {
        U256::from(value).to_big_endian()
    }

    fn finalized_status_fact(
        input_index: u64,
        hash: u8,
        in_non_finalized_cache: bool,
    ) -> FinalizedTransactionStatusFact {
        FinalizedTransactionStatusFact {
            input_index,
            hash: [hash; 32],
            in_non_finalized_cache,
        }
    }

    fn finalized_filter_fact(
        input_index: u64,
        hash: u8,
        in_recently_finalized_cache: bool,
    ) -> TransactionManagerFinalizedFilterFact {
        TransactionManagerFinalizedFilterFact {
            input_index,
            hash: [hash; 32],
            in_recently_finalized_cache,
        }
    }

    fn verify_not_finalized_fact(
        input_index: u64,
        hash: u8,
        transaction_nonce: u64,
        sender_account_nonce: u64,
        in_recently_finalized_cache: bool,
    ) -> TransactionManagerVerifyNotFinalizedFact {
        TransactionManagerVerifyNotFinalizedFact {
            input_index,
            hash: [hash; 32],
            transaction_nonce: u256_bytes(transaction_nonce),
            sender_account_nonce: u256_bytes(sender_account_nonce),
            in_recently_finalized_cache,
        }
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

    fn validated_insert_fact(
        tx_hash: u8,
        account_found: bool,
        account_nonce: u64,
        account_balance: u64,
        insert_non_proposable: bool,
    ) -> TransactionManagerValidatedInsertFact {
        TransactionManagerValidatedInsertFact {
            tx_hash: [tx_hash; 32],
            transaction_nonce: u256_bytes(1),
            transaction_cost: u256_bytes(10),
            gas_limit: 21_000,
            propose_dag_gas_limit: 100_000,
            insert_non_proposable,
            in_non_finalized_cache: false,
            in_recently_finalized_cache: false,
            account_found,
            account_nonce: u256_bytes(account_nonce),
            account_balance: u256_bytes(account_balance),
        }
    }

    fn validated_insert_sidecar_fact(
        tx_hash: u8,
        account_found: bool,
        account_nonce: u64,
        account_balance: u64,
        insert_non_proposable: bool,
    ) -> TransactionManagerValidatedInsertSidecarFact {
        TransactionManagerValidatedInsertSidecarFact {
            tx_hash: [tx_hash; 32],
            transaction_nonce: u256_bytes(1),
            transaction_cost: u256_bytes(10),
            gas_limit: 21_000,
            propose_dag_gas_limit: 100_000,
            insert_non_proposable,
            account_found,
            account_nonce: u256_bytes(account_nonce),
            account_balance: u256_bytes(account_balance),
        }
    }

    fn validated_insert_runtime_fact(
        tx_hash: u8,
        sender: [u8; 20],
        insert_non_proposable: bool,
    ) -> TransactionManagerValidatedInsertRuntimeFact {
        TransactionManagerValidatedInsertRuntimeFact {
            tx_hash: [tx_hash; 32],
            sender,
            transaction_nonce: u256_bytes(1),
            transaction_cost: u256_bytes(10),
            gas_limit: 21_000,
            propose_dag_gas_limit: 100_000,
            insert_non_proposable,
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

    fn verify_not_finalized_runtime_fact(
        input_index: u64,
        hash: u8,
        transaction_nonce: u64,
        sender: [u8; 20],
    ) -> TransactionManagerVerifyNotFinalizedRuntimeFact {
        TransactionManagerVerifyNotFinalizedRuntimeFact {
            input_index,
            hash: [hash; 32],
            transaction_nonce: u256_bytes(transaction_nonce),
            sender,
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
    fn bridge_insert_transaction_with_sidecar_combines_queue_known_and_sidecar_membership() {
        let mut sidecar = create_transaction_manager_sidecar(5);
        sidecar
            .transaction_manager_sidecar_insert_non_finalized(
                TransactionManagerSidecarInsertInput {
                    hash: [7; 32],
                    trx_rlp: vec![0x07],
                },
            )
            .expect("sidecar insert should succeed");

        let known = transaction_manager_insert_transaction_with_sidecar(
            &sidecar,
            insert_fact(
                7,
                false,
                rustaxa_consensus::transaction_queue::TransactionQueueInsertStatus::Inserted as u8,
                false,
                0,
            ),
        )
        .expect("insert-with-sidecar plan should compute");
        assert_eq!(known.status, TM_INSERT_TRANSACTION_STATUS_ALREADY_KNOWN);

        let finalized = transaction_manager_insert_transaction_with_sidecar(
            &sidecar,
            insert_fact(
                8,
                false,
                rustaxa_consensus::transaction_queue::TransactionQueueInsertStatus::Known as u8,
                true,
                42,
            ),
        )
        .expect("insert-with-sidecar plan should compute");
        assert_eq!(
            finalized.status,
            TM_INSERT_TRANSACTION_STATUS_ALREADY_FINALIZED
        );
        assert!(finalized.finalized_period_known);
        assert_eq!(finalized.finalized_period, 42);
    }

    #[test]
    fn bridge_transaction_manager_plan_validated_insert_returns_queue_plan() {
        let proposable = transaction_manager_plan_validated_insert(validated_insert_fact(
            1, true, 0, 100, false,
        ))
        .expect("validated insert plan should compute");
        assert_eq!(
            proposable.queue_action,
            TM_VALIDATED_INSERT_QUEUE_ACTION_INSERT_PROPOSABLE
        );
        assert!(proposable.emit_transaction_added);
        assert_eq!(
            proposable.status,
            rustaxa_consensus::transaction_queue::TransactionQueueInsertStatus::Inserted as u8
        );

        let non_proposable =
            transaction_manager_plan_validated_insert(validated_insert_fact(2, false, 0, 0, true))
                .expect("validated insert plan should compute");
        assert_eq!(
            non_proposable.queue_action,
            TM_VALIDATED_INSERT_QUEUE_ACTION_INSERT_NON_PROPOSABLE
        );
        assert_eq!(
            non_proposable.status,
            rustaxa_consensus::transaction_queue::TransactionQueueInsertStatus::InsertedNonProposable as u8
        );
        assert!(!non_proposable.emit_transaction_added);

        let rejected =
            transaction_manager_plan_validated_insert(validated_insert_fact(3, false, 0, 0, false))
                .expect("validated insert plan should compute");
        assert_eq!(rejected.queue_action, TM_VALIDATED_INSERT_QUEUE_ACTION_NONE);
        assert_eq!(
            rejected.status,
            rustaxa_consensus::transaction_queue::TransactionQueueInsertStatus::Known as u8
        );
        assert!(!rejected.emit_transaction_added);
    }

    #[test]
    fn bridge_transaction_manager_filter_non_finalized_skips_recent_and_finalized_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_transaction_manager_filter_non_finalized");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .save_transaction_location(&[2u8; 32], 7, 0, false)
            .expect("finalized hash should be persisted in trx period");

        let out = transaction_manager_filter_non_finalized(
            &storage,
            vec![
                finalized_filter_fact(0, 1, false),
                finalized_filter_fact(1, 2, false),
                finalized_filter_fact(2, 3, true),
            ],
        )
        .expect("filtering plan should map finalized inputs");

        assert_eq!(out.not_finalized.len(), 1);
        assert_eq!(out.not_finalized[0].input_index, 0);
        assert_eq!(out.not_finalized[0].hash, [1; 32]);

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_verify_not_finalized_honors_recent_cache_before_storage() {
        let temp_dir =
            unique_temp_dir("rustaxa_bridge_transaction_manager_verify_not_finalized_cache");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .save_transaction_location(&[2u8; 32], 7, 0, false)
            .expect("finalized hash should be persisted in trx period");

        let out = transaction_manager_verify_not_finalized(
            &storage,
            vec![
                verify_not_finalized_fact(0, 1, 2, 8, true),
                verify_not_finalized_fact(1, 2, 1, 2, false),
            ],
        )
        .expect("verify plan should short-circuit on cache");

        assert!(out.is_finalized);
        assert_eq!(out.input_index, 0);
        assert_eq!(out.hash, [1; 32]);

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_verify_not_finalized_performs_gated_storage_lookup() {
        let temp_dir =
            unique_temp_dir("rustaxa_bridge_transaction_manager_verify_not_finalized_lookup");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .save_transaction_location(&[2u8; 32], 7, 0, false)
            .expect("finalized hash should be persisted in trx period");

        let out = transaction_manager_verify_not_finalized(
            &storage,
            vec![
                verify_not_finalized_fact(0, 1, 10, 1, false),
                verify_not_finalized_fact(1, 2, 2, 4, false),
            ],
        )
        .expect("verify plan should fail on finalized input");

        assert!(out.is_finalized);
        assert_eq!(out.input_index, 1);
        assert_eq!(out.hash, [2; 32]);

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_save_transactions_from_dag_block_persists_accepted_hashes_and_count() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_transaction_manager_save_dag");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .save_status_field(StatusField::TrxCount as u8, 7)
            .expect("status field seed should persist");
        storage
            .save_transaction_location(&[4; 32], 1, 0, false)
            .expect("finalized transaction location should persist");

        let out = save_transactions_from_dag_block(
            &storage,
            7,
            vec![
                dag_tx_fact(0, 1, 5, 4, false, false, 11),
                dag_tx_fact(1, 1, 5, 4, false, false, 12),
                dag_tx_fact(2, 2, 5, 4, true, false, 21),
                dag_tx_fact(3, 3, 5, 4, false, true, 31),
                dag_tx_fact(4, 4, 5, 11, false, false, 41),
                dag_tx_fact(5, 5, 2, 1, false, false, 51),
            ],
        )
        .expect("dag transactions from block should save");

        assert_eq!(
            out.accepted
                .iter()
                .map(|entry| (entry.input_index, entry.hash))
                .collect::<Vec<_>>(),
            vec![(0, [1; 32]), (5, [5; 32])]
        );
        assert_eq!(out.target_transaction_count, 9);

        assert_eq!(
            storage
                .get_status_field(StatusField::TrxCount as u8)
                .expect("status field should persist"),
            9,
        );
        assert_eq!(
            storage
                .get_transaction(&[1u8; 32])
                .expect("transaction should persist"),
            vec![11]
        );
        assert_eq!(
            storage
                .get_transaction(&[5u8; 32])
                .expect("transaction should persist"),
            vec![51]
        );
        assert_eq!(
            storage
                .get_transaction(&[2u8; 32])
                .expect("non-accepted transaction should be missing"),
            Vec::<u8>::new()
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_save_transactions_from_dag_block_skips_all_when_no_new_accepts() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_transaction_manager_save_dag_skip");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .save_status_field(StatusField::TrxCount as u8, 7)
            .expect("status field seed should persist");
        storage
            .save_transaction_location(&[2; 32], 1, 0, false)
            .expect("finalized transaction location should persist");

        let out = save_transactions_from_dag_block(
            &storage,
            7,
            vec![
                dag_tx_fact(0, 1, 5, 4, true, false, 11),
                dag_tx_fact(1, 2, 5, 8, false, false, 21),
                dag_tx_fact(2, 3, 5, 4, false, true, 31),
            ],
        )
        .expect("skip-only list should not fail");

        assert!(out.accepted.is_empty());
        assert_eq!(out.target_transaction_count, 7);
        assert_eq!(
            storage
                .get_status_field(StatusField::TrxCount as u8)
                .expect("status field should remain"),
            7
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_admission_execute_is_side_effect_free_until_commit() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_admission_execute");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let mut runtime =
            create_transaction_manager_runtime(7, TransactionQueueConfig { max_size: 16 });
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input(1, true))
            .expect("queue seed should succeed");

        let execution = transaction_manager_runtime_execute_admission(
            &runtime,
            &storage,
            vec![dag_tx_sidecar_fact(0, 1, 5, 4, 0x11)],
        )
        .expect("runtime admission execute should succeed");

        assert!(runtime.transaction_manager_runtime_queue_contains(&[1; 32]));
        assert!(!runtime.transaction_manager_runtime_contains_non_finalized(&[1; 32]));
        assert_eq!(
            storage
                .get_transaction(&[1; 32])
                .expect("storage read should succeed"),
            Vec::<u8>::new()
        );

        let out = transaction_manager_runtime_commit_admission(&mut runtime, &storage, execution)
            .expect("runtime admission commit should succeed");
        assert_eq!(out.accepted.len(), 1);
        assert!(out.accepted[0].erased_from_queue);
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[1; 32]));
        assert!(runtime.transaction_manager_runtime_contains_non_finalized(&[1; 32]));
        assert_eq!(
            storage
                .get_transaction(&[1; 32])
                .expect("storage write should persist"),
            vec![0x11]
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_save_transactions_from_dag_block_with_runtime_uses_admission_commit_path() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_admission_wrapper");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let mut runtime =
            create_transaction_manager_runtime(7, TransactionQueueConfig { max_size: 16 });
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input(1, true))
            .expect("queue seed should succeed");

        let out = save_transactions_from_dag_block_with_runtime(
            &mut runtime,
            &storage,
            vec![dag_tx_sidecar_fact(0, 1, 5, 4, 0x33)],
        )
        .expect("runtime wrapper should succeed");

        assert_eq!(out.accepted.len(), 1);
        assert!(out.accepted[0].erased_from_queue);
        assert_eq!(out.target_transaction_count, 8);
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[1; 32]));
        assert!(runtime.transaction_manager_runtime_contains_non_finalized(&[1; 32]));
        assert_eq!(
            storage
                .get_status_field(StatusField::TrxCount as u8)
                .expect("status field should persist"),
            8
        );

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_save_transactions_from_dag_block_with_runtime_and_final_chain_sources_sender_nonce() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_admission_fc");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let sender = [8; 20];
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
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("final chain should initialize");
        let mut runtime =
            create_transaction_manager_runtime(7, TransactionQueueConfig { max_size: 16 });
        let out = save_transactions_from_dag_block_with_runtime_and_final_chain(
            &mut runtime,
            &storage,
            &final_chain,
            vec![dag_tx_runtime_fact(0, 1, 1, sender, 0x33)],
        )
        .expect("runtime final-chain DAG admission should succeed");
        assert_eq!(out.accepted.len(), 1);
        assert_eq!(out.target_transaction_count, 8);
        assert_eq!(
            storage
                .get_transaction(&[1; 32])
                .expect("accepted transaction should persist"),
            vec![0x33]
        );
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_save_transactions_from_dag_block_command_report_with_runtime_and_final_chain_maps_actions(
    ) {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_admission_fc_command_report");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("final chain should initialize");
        let sender = [8; 20];
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
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("final chain should initialize");
        let mut runtime =
            create_transaction_manager_runtime(7, TransactionQueueConfig { max_size: 16 });
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input(1, true))
            .expect("queue seed should succeed");

        let report = save_transactions_from_dag_block_command_report_with_runtime_and_final_chain(
            &mut runtime,
            &storage,
            &final_chain,
            vec![dag_tx_runtime_fact(0, 1, 1, sender, 0x33)],
        )
        .expect("runtime final-chain DAG command report should execute");

        assert_eq!(report.queue_erased.len(), 1);
        assert_eq!(report.queue_erased[0].hash, [1; 32]);
        assert_eq!(runtime.transaction_manager_runtime_transaction_count(), 8);
        assert_eq!(
            storage
                .get_status_field(StatusField::TrxCount as u8)
                .expect("status field should persist"),
            8
        );
        assert_eq!(
            storage
                .get_transaction(&[1; 32])
                .expect("accepted transaction should persist"),
            vec![0x33]
        );
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_update_finalized_transactions_status_plans_and_persists_count_when_non_empty() {
        let temp_dir =
            unique_temp_dir("rustaxa_bridge_transaction_manager_update_finalized_status");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .save_status_field(StatusField::TrxCount as u8, 7)
            .expect("status field seed should persist");

        let out = update_finalized_transactions_status(
            &storage,
            200,
            10,
            7,
            vec![
                finalized_status_fact(0, 1, false),
                finalized_status_fact(1, 2, true),
                finalized_status_fact(2, 3, false),
            ],
        )
        .expect("finalized status plan should be computed");

        assert_eq!(
            out.accepted
                .iter()
                .map(|entry| (entry.input_index, entry.hash))
                .collect::<Vec<_>>(),
            vec![(0, [1; 32]), (1, [2; 32]), (2, [3; 32])]
        );
        assert_eq!(out.target_transaction_count, 9);
        assert_eq!(out.stale_period, 190);
        assert!(out.has_stale_period);
        assert!(out.purge_transaction_queue);
        assert_eq!(
            storage
                .get_status_field(StatusField::TrxCount as u8)
                .expect("status field should persist"),
            9,
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
            .save_status_field(StatusField::TrxCount as u8, 7)
            .expect("status field seed should persist");

        let mut runtime =
            create_transaction_manager_runtime(7, TransactionQueueConfig { max_size: 16 });
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
            &storage,
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
            storage
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
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("final chain should initialize");

        let mut runtime =
            create_transaction_manager_runtime(7, TransactionQueueConfig { max_size: 16 });
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input_for_sender(
                1, sender, 1, true,
            ))
            .expect("queue seed should succeed");

        let report =
            update_finalized_transactions_status_command_report_with_runtime_and_final_chain(
                &mut runtime,
                &storage,
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
    fn bridge_update_finalized_transactions_status_skips_status_persistence_when_no_inputs() {
        let temp_dir =
            unique_temp_dir("rustaxa_bridge_transaction_manager_update_finalized_status_skip");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");

        storage
            .save_status_field(StatusField::TrxCount as u8, 11)
            .expect("status field seed should persist");

        let out = update_finalized_transactions_status(&storage, 200, 10, 11, vec![])
            .expect("empty finalized list should still plan");

        assert!(out.accepted.is_empty());
        assert_eq!(out.target_transaction_count, 11);
        assert_eq!(
            storage
                .get_status_field(StatusField::TrxCount as u8)
                .expect("status field should remain unchanged"),
            11,
        );

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
            .save_transaction(&[1u8; 32], vec![0x11])
            .expect("pending transaction should persist");

        // Persist finalized location metadata and tx-by-position data for hash 2 so lookup
        // exercises finalized fallback path after non-finalized miss.
        storage
            .save_transaction_location(&[2u8; 32], 8, 0, false)
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
            .save_period_data(8, period_data.out().as_ref().to_vec())
            .expect("period data should persist");
        storage
            .save_system_transaction(&[4u8; 32], vec![0x44])
            .expect("system transaction should persist");
        storage
            .save_transaction_location(&[4u8; 32], 9, 0, true)
            .expect("system finalized location should persist");

        let out = transaction_manager_load_stored_transactions(
            &storage,
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
            .map(|entry| {
                (
                    entry.input_index,
                    entry.hash,
                    entry.found,
                    entry.source,
                    entry.tx_rlp,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(out.len(), 4);
        assert_eq!(
            out[0],
            (
                7,
                [2u8; 32],
                true,
                TM_STORED_TX_SOURCE_FINALIZED_REGULAR,
                vec![0x22]
            )
        );
        assert_eq!(
            out[1],
            (
                8,
                [3u8; 32],
                false,
                TM_STORED_TX_SOURCE_MISSING,
                Vec::<u8>::new()
            )
        );
        assert_eq!(
            out[2],
            (9, [1u8; 32], true, TM_STORED_TX_SOURCE_PENDING, vec![0x11])
        );
        assert_eq!(
            out[3],
            (
                10,
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
            .save_transaction(&[1u8; 32], vec![0x11])
            .expect("non-finalized transaction should persist");
        storage
            .save_transaction(&[2u8; 32], vec![0x22])
            .expect("finalized stale entry should persist");
        storage
            .save_transaction_location(&[2u8; 32], 11, 0, false)
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
            .save_period_data(11, period_data.out().as_ref().to_vec())
            .expect("period data should persist");

        let out = transaction_manager_load_nonfinalized_recovery(&storage)
            .expect("recovery payload lookup should inspect all non-finalized storage rows");

        assert_eq!(out.len(), 2);
        let mut by_hash = out
            .into_iter()
            .map(|entry| (entry.hash[0], entry.finalized))
            .collect::<Vec<_>>();
        by_hash.sort_unstable();
        assert_eq!(by_hash, vec![(1u8, false), (2u8, true)]);
        assert_eq!(
            storage
                .get_transaction(&[2u8; 32])
                .expect("stale finalized entry should be removed"),
            Vec::<u8>::new()
        );
        assert_eq!(
            storage
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
            .save_transaction(&envelope.hash.0, tx_rlp.clone())
            .expect("non-finalized transaction should persist");

        let inputs = transaction_manager_load_nonfinalized_recovery_inputs(&storage)
            .expect("recovery inputs should validate live survivor envelopes");

        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].hash, envelope.hash.0);
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
            .save_transaction(&live_hash, live_tx.clone())
            .expect("non-finalized transaction should persist");
        storage
            .save_transaction(&[2u8; 32], vec![0x22])
            .expect("stale transaction should persist");
        storage
            .save_transaction_location(&[2u8; 32], 11, 0, false)
            .expect("stale finalized location should persist");

        let mut runtime =
            create_transaction_manager_runtime(4, TransactionQueueConfig { max_size: 16 });

        transaction_manager_recover_nonfinalized_with_runtime(&mut runtime, &storage)
            .expect("runtime recovery should execute");

        assert_eq!(runtime.transaction_manager_runtime_transaction_count(), 4);
        assert!(runtime.transaction_manager_runtime_contains_non_finalized(&live_hash));
        assert_eq!(
            storage
                .get_transaction(&[2u8; 32])
                .expect("stale tx should be removed"),
            Vec::<u8>::new()
        );
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_sidecar_supports_lookup_finalize_evict_and_recovery() {
        let mut sidecar = create_transaction_manager_sidecar(12);
        assert_eq!(sidecar.transaction_manager_sidecar_transaction_count(), 12);
        sidecar
            .transaction_manager_sidecar_insert_non_finalized(
                TransactionManagerSidecarInsertInput {
                    hash: [1; 32],
                    trx_rlp: vec![0x11],
                },
            )
            .unwrap();
        sidecar
            .transaction_manager_sidecar_insert_non_finalized(
                TransactionManagerSidecarInsertInput {
                    hash: [2; 32],
                    trx_rlp: vec![0x22],
                },
            )
            .unwrap();
        assert!(sidecar
            .transaction_manager_sidecar_is_transaction_known(TransactionManagerSidecarKnownFact {
                hash: [1; 32],
                queue_known: false,
            },)
            .unwrap());
        assert!(sidecar
            .transaction_manager_sidecar_is_transaction_known(TransactionManagerSidecarKnownFact {
                hash: [9; 32],
                queue_known: true,
            },)
            .unwrap());
        assert!(!sidecar
            .transaction_manager_sidecar_is_transaction_known(TransactionManagerSidecarKnownFact {
                hash: [8; 32],
                queue_known: false,
            },)
            .unwrap());

        let lookup = sidecar
            .transaction_manager_sidecar_lookup_ordered_payloads(vec![
                TransactionManagerSidecarLookupRequest {
                    input_index: 3,
                    hash: [2; 32],
                },
                TransactionManagerSidecarLookupRequest {
                    input_index: 4,
                    hash: [9; 32],
                },
            ])
            .unwrap();
        assert_eq!(lookup.lookups.len(), 2);
        assert!(lookup.lookups[0].found);
        assert_eq!(
            lookup.lookups[0].source,
            rustaxa_consensus::transaction_manager::TransactionManagerSidecarLookup::SOURCE_NON_FINALIZED
        );
        assert_eq!(lookup.lookups[0].trx_rlp, vec![0x22]);
        assert!(!lookup.lookups[1].found);
        assert_eq!(
            lookup.lookups[1].source,
            rustaxa_consensus::transaction_manager::TransactionManagerSidecarLookup::SOURCE_MISSING
        );

        sidecar
            .transaction_manager_sidecar_apply_finalized_transition(
                TransactionManagerSidecarTransitionInput {
                    period: 55,
                    hashes: vec![crate::ffi::rustaxa_ffi::TransactionManagerSidecarHash {
                        hash: [1; 32],
                    }],
                },
            )
            .unwrap();
        assert!(!sidecar.transaction_manager_sidecar_contains_non_finalized(&[1; 32]));
        assert!(sidecar.transaction_manager_sidecar_contains_recently_finalized(&[1; 32]));
        assert_eq!(
            sidecar.transaction_manager_sidecar_evict_stale_recently_finalized(55),
            1
        );
        assert!(!sidecar.transaction_manager_sidecar_contains_recently_finalized(&[1; 32]));

        let inserted = sidecar
            .transaction_manager_sidecar_insert_recovery_entries(vec![
                TransactionManagerSidecarRecoveryInsertInput {
                    hash: [3; 32],
                    finalized: false,
                    trx_rlp: vec![0x33],
                },
                TransactionManagerSidecarRecoveryInsertInput {
                    hash: [4; 32],
                    finalized: true,
                    trx_rlp: vec![0x44],
                },
            ])
            .unwrap();
        assert_eq!(inserted, 1);
        assert!(sidecar.transaction_manager_sidecar_contains_non_finalized(&[3; 32]));
        assert!(!sidecar.transaction_manager_sidecar_contains_non_finalized(&[4; 32]));
    }

    #[test]
    fn bridge_transaction_manager_runtime_insert_validated_executes_queue_insert() {
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        let outcome = runtime
            .transaction_manager_runtime_insert_validated_transaction(
                validated_insert_sidecar_fact(5, true, 0, 100, false),
                runtime_queue_input(5, false),
            )
            .expect("runtime validated insert should succeed");
        assert_eq!(
            outcome.status,
            rustaxa_consensus::transaction_queue::TransactionQueueInsertStatus::Inserted as u8
        );
        assert!(outcome.emit_transaction_added);
        assert!(outcome.inserted_hash_found);
        assert!(runtime.transaction_manager_runtime_queue_contains(&[5; 32]));
    }

    #[test]
    fn bridge_transaction_manager_runtime_insert_validated_short_circuits_known() {
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        runtime
            .transaction_manager_runtime_insert_non_finalized(
                TransactionManagerSidecarInsertInput {
                    hash: [7; 32],
                    trx_rlp: vec![0x07],
                },
            )
            .expect("runtime sidecar insert should succeed");

        let outcome = runtime
            .transaction_manager_runtime_insert_validated_transaction(
                validated_insert_sidecar_fact(7, true, 0, 100, false),
                runtime_queue_input(7, true),
            )
            .expect("runtime known short-circuit should succeed");
        assert_eq!(
            outcome.status,
            rustaxa_consensus::transaction_queue::TransactionQueueInsertStatus::Known as u8
        );
        assert!(!outcome.emit_transaction_added);
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[7; 32]));
    }

    #[test]
    fn bridge_transaction_manager_runtime_insert_precheck_uses_runtime_known_state() {
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        let initial = runtime
            .transaction_manager_runtime_insert_transaction_precheck(&[12; 32])
            .expect("insert precheck should succeed");
        assert_eq!(initial.status, TM_INSERT_TRANSACTION_STATUS_ACCEPTED);

        let admission = runtime
            .transaction_manager_runtime_execute_transaction_admission(
                validated_insert_sidecar_fact(12, true, 0, 100, false),
                runtime_queue_input(12, false),
                false,
                0,
            )
            .expect("runtime admission should insert");
        assert_eq!(
            admission.transaction_status,
            rustaxa_consensus::transaction_queue::TransactionQueueInsertStatus::Inserted as u8
        );

        let known = runtime
            .transaction_manager_runtime_insert_transaction_precheck(&[12; 32])
            .expect("insert precheck should see known queue state");
        assert_eq!(known.status, TM_INSERT_TRANSACTION_STATUS_ALREADY_KNOWN);
    }

    #[test]
    fn bridge_transaction_manager_runtime_admission_requests_finalized_finish_for_known_status() {
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        let admission = runtime
            .transaction_manager_runtime_execute_transaction_admission(
                validated_insert_sidecar_fact(13, false, 0, 0, false),
                runtime_queue_input(13, false),
                false,
                0,
            )
            .expect("runtime admission should return status without queue mutation");
        assert_eq!(
            admission.transaction_status,
            rustaxa_consensus::transaction_queue::TransactionQueueInsertStatus::Known as u8
        );
        assert!(admission.requires_finalized_lookup);
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[13; 32]));

        let finished = runtime
            .transaction_manager_runtime_finish_insert_transaction(
                TransactionManagerInsertTransactionFact {
                    tx_hash: [13; 32],
                    hash_known: false,
                    queue_status: admission.transaction_status,
                    has_finalized_period: true,
                    finalized_period: 22,
                },
            )
            .expect("runtime finish should map finalized fact");
        assert_eq!(
            finished.status,
            TM_INSERT_TRANSACTION_STATUS_ALREADY_FINALIZED
        );
        assert!(finished.finalized_period_known);
        assert_eq!(finished.finalized_period, 22);
    }

    #[test]
    fn bridge_transaction_manager_runtime_admission_with_storage_sets_finalized_period() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_admission_storage");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        storage
            .save_transaction_location(&[13u8; 32], 33, 0, false)
            .expect("finalized location should persist");
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input(13, true))
            .expect("runtime queue insert should succeed");

        let admission = runtime
            .transaction_manager_runtime_execute_transaction_admission_with_storage(
                &storage,
                validated_insert_sidecar_fact(13, true, 0, 100, false),
                runtime_queue_input(13, false),
            )
            .expect("runtime admission with storage should execute");

        assert_eq!(
            admission.insert_status,
            TM_INSERT_TRANSACTION_STATUS_ALREADY_FINALIZED
        );
        assert!(!admission.requires_finalized_lookup);
        assert!(admission.finalized_period_known);
        assert_eq!(admission.finalized_period, 33);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_runtime_admission_with_storage_completes_missing_location() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_admission_storage_lookup");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input(14, true))
            .expect("runtime queue insert should succeed");

        let admission = runtime
            .transaction_manager_runtime_execute_transaction_admission_with_storage(
                &storage,
                validated_insert_sidecar_fact(14, true, 0, 100, false),
                runtime_queue_input(14, false),
            )
            .expect("runtime admission with storage lookup should execute");

        assert_eq!(
            admission.insert_status,
            TM_INSERT_TRANSACTION_STATUS_CANNOT_INSERT
        );
        assert!(!admission.requires_finalized_lookup);
        assert!(!admission.finalized_period_known);
        assert_eq!(admission.finalized_period, 0);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_runtime_admission_with_storage_command_report_includes_finalized_status(
    ) {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_admission_storage_report");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        storage
            .save_transaction_location(&[16u8; 32], 17, 0, false)
            .expect("finalized location should persist");
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input(16, true))
            .expect("runtime queue insert should succeed");

        let report = runtime
            .transaction_manager_runtime_execute_transaction_admission_with_storage_command_report(
                &storage,
                validated_insert_sidecar_fact(16, true, 0, 100, false),
                runtime_queue_input(16, false),
            )
            .expect("runtime admission storage report should execute");

        assert!(report.admission.present);
        assert_eq!(
            report.admission.insert_status,
            TM_INSERT_TRANSACTION_STATUS_ALREADY_FINALIZED
        );
        assert!(report.admission.finalized_period_known);
        assert_eq!(report.admission.finalized_period, 17);
        assert!(!report.inserted_hash_found);
        assert!(!report.transaction_added_hash_found);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_runtime_admission_with_final_chain_sets_finalized_period() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_admission_period_fc");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let sender = [6; 20];
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
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("final chain should initialize");
        storage
            .save_transaction_location(&[13u8; 32], 22, 0, false)
            .expect("finalized location should persist");
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        let admission = runtime
            .transaction_manager_runtime_execute_transaction_admission_with_final_chain(
                &final_chain,
                validated_insert_runtime_fact(13, sender, false),
                runtime_queue_input(13, false),
            )
            .expect("runtime admission with final chain should execute");
        assert_eq!(
            admission.insert_status,
            TM_INSERT_TRANSACTION_STATUS_ALREADY_FINALIZED
        );
        assert!(!admission.requires_finalized_lookup);
        assert!(admission.finalized_period_known);
        assert_eq!(admission.finalized_period, 22);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_runtime_admission_command_report_maps_actions() {
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        let report = runtime
            .transaction_manager_runtime_execute_transaction_admission_command_report(
                validated_insert_sidecar_fact(14, true, 0, 100, false),
                runtime_queue_input(14, false),
                false,
                0,
            )
            .expect("runtime admission command report should execute");

        assert!(report.admission.present);
        assert_eq!(
            report.admission.insert_status,
            TM_INSERT_TRANSACTION_STATUS_ACCEPTED
        );
        assert_eq!(
            report.admission.transaction_status,
            TransactionQueueInsertStatus::Inserted as u8
        );
        assert!(!report.admission.finalized_period_known);
        assert_eq!(report.admission.finalized_period, 0);
        assert!(!report.admission.requires_finalized_lookup);
        assert!(report.inserted_hash_found);
        assert_eq!(report.inserted_hash, [14; 32]);
        assert!(report.transaction_added_hash_found);
        assert_eq!(report.transaction_added_hash, [14; 32]);
        assert!(runtime.transaction_manager_runtime_queue_contains(&[14; 32]));
    }

    #[test]
    fn bridge_transaction_manager_runtime_admission_with_final_chain_command_report_includes_status(
    ) {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_admission_fc_report");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let sender = [6; 20];
        let final_chain = crate::final_chain::create_final_chain(
            &storage,
            1_000_000,
            1,
            vec![crate::ffi::rustaxa_ffi::GenesisAccount {
                address: sender,
                balance: vec![20],
            }],
            Vec::new(),
            crate::ffi::rustaxa_ffi::GenesisDposConfig {
                eligibility_balance_threshold: vec![1],
                vote_eligibility_balance_step: vec![1],
                validator_maximum_stake: vec![1],
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("final chain should initialize");
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        let report = runtime
            .transaction_manager_runtime_execute_transaction_admission_with_final_chain_command_report(
                &final_chain,
                validated_insert_runtime_fact(15, sender, false),
                runtime_queue_input(15, false),
            )
            .expect("runtime admission final-chain report should execute");

        assert!(report.admission.present);
        assert_eq!(
            report.admission.insert_status,
            TM_INSERT_TRANSACTION_STATUS_ACCEPTED
        );
        assert_eq!(
            report.admission.transaction_status,
            TransactionQueueInsertStatus::Inserted as u8
        );
        assert!(report.inserted_hash_found);
        assert_eq!(report.inserted_hash, [15; 32]);
        assert!(report.transaction_added_hash_found);
        assert_eq!(report.transaction_added_hash, [15; 32]);
        assert!(runtime.transaction_manager_runtime_queue_contains(&[15; 32]));
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_runtime_public_admission_command_report_accepts_transaction() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_public_admission_accept");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let sender = [6; 20];
        let final_chain = crate::final_chain::create_final_chain(
            &storage,
            1_000_000,
            1,
            vec![crate::ffi::rustaxa_ffi::GenesisAccount {
                address: sender,
                balance: vec![20],
            }],
            Vec::new(),
            crate::ffi::rustaxa_ffi::GenesisDposConfig {
                eligibility_balance_threshold: vec![1],
                vote_eligibility_balance_step: vec![1],
                validator_maximum_stake: vec![1],
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("final chain should initialize");
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        let report = runtime
            .transaction_manager_runtime_execute_public_transaction_admission_with_final_chain_command_report(
                &final_chain,
                verify_fact(17, 1, 1, 21_000, 100_000, false, true, true, 2, 1, 1),
                validated_insert_runtime_fact(17, sender, false),
                runtime_queue_input_for_sender(17, sender, 1, false),
            )
            .expect("runtime public admission report should execute");

        assert_eq!(
            report.verification_status,
            TM_VERIFY_TRANSACTION_STATUS_ACCEPTED
        );
        assert_eq!(report.verification_chain_id, 1);
        assert_eq!(report.verification_expected_chain_id, 1);
        assert!(report.public_result.accepted);
        assert_eq!(report.public_result.message, "");
        assert!(report.admission.admission.present);
        assert_eq!(
            report.admission.admission.insert_status,
            TM_INSERT_TRANSACTION_STATUS_ACCEPTED
        );
        assert_eq!(
            report.admission.admission.transaction_status,
            TransactionQueueInsertStatus::Inserted as u8
        );
        assert!(report.admission.inserted_hash_found);
        assert_eq!(report.admission.inserted_hash, [17; 32]);
        assert!(report.admission.transaction_added_hash_found);
        assert_eq!(report.admission.transaction_added_hash, [17; 32]);
        assert!(runtime.transaction_manager_runtime_queue_contains(&[17; 32]));
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_runtime_public_admission_command_report_short_circuits_known() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_public_admission_known");
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
                balance: vec![20],
            }],
            Vec::new(),
            crate::ffi::rustaxa_ffi::GenesisDposConfig {
                eligibility_balance_threshold: vec![1],
                vote_eligibility_balance_step: vec![1],
                validator_maximum_stake: vec![1],
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("final chain should initialize");
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input_for_sender(
                18, sender, 1, true,
            ))
            .expect("queue insert should succeed");

        let report = runtime
            .transaction_manager_runtime_execute_public_transaction_admission_with_final_chain_command_report(
                &final_chain,
                verify_fact(18, 9, 1, 21_000, 100_000, false, true, true, 2, 1, 1),
                validated_insert_runtime_fact(18, sender, false),
                runtime_queue_input_for_sender(18, sender, 1, false),
            )
            .expect("runtime public known report should execute");

        assert_eq!(
            report.verification_status,
            TM_VERIFY_TRANSACTION_STATUS_ACCEPTED
        );
        assert!(!report.public_result.accepted);
        assert_eq!(
            report.public_result.message,
            "Transaction already in transactions pool"
        );
        assert!(report.admission.admission.present);
        assert_eq!(
            report.admission.admission.insert_status,
            TM_INSERT_TRANSACTION_STATUS_ALREADY_KNOWN
        );
        assert_eq!(report.admission.admission.transaction_status, 0);
        assert!(!report.admission.inserted_hash_found);
        assert!(!report.admission.transaction_added_hash_found);
        assert!(runtime.transaction_manager_runtime_queue_contains(&[18; 32]));
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_runtime_public_admission_command_report_returns_verify_message() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_public_admission_verify_reject");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let sender = [8; 20];
        let final_chain = crate::final_chain::create_final_chain(
            &storage,
            1_000_000,
            1,
            vec![crate::ffi::rustaxa_ffi::GenesisAccount {
                address: sender,
                balance: vec![20],
            }],
            Vec::new(),
            crate::ffi::rustaxa_ffi::GenesisDposConfig {
                eligibility_balance_threshold: vec![1],
                vote_eligibility_balance_step: vec![1],
                validator_maximum_stake: vec![1],
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("final chain should initialize");
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });

        let report = runtime
            .transaction_manager_runtime_execute_public_transaction_admission_with_final_chain_command_report(
                &final_chain,
                verify_fact(19, 9, 1, 21_000, 100_000, false, true, true, 2, 1, 1),
                validated_insert_runtime_fact(19, sender, false),
                runtime_queue_input_for_sender(19, sender, 1, false),
            )
            .expect("runtime public verify rejection report should execute");

        assert_eq!(
            report.verification_status,
            TM_VERIFY_TRANSACTION_STATUS_CHAIN_ID_MISMATCH
        );
        assert!(!report.public_result.accepted);
        assert_eq!(report.public_result.message, "chain_id mismatch 9 1");
        assert!(!report.admission.admission.present);
        assert!(!report.admission.inserted_hash_found);
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[19; 32]));
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_transaction_manager_runtime_insert_validated_rejects_hash_mismatch() {
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        let err = match runtime.transaction_manager_runtime_insert_validated_transaction(
            validated_insert_sidecar_fact(8, true, 0, 100, false),
            runtime_queue_input(9, true),
        ) {
            Ok(_) => panic!("runtime validated insert should reject mismatched hash"),
            Err(err) => err,
        };
        assert!(err
            .to_string()
            .contains("TM_RUNTIME_VALIDATED_INSERT_HASH_MISMATCH"));
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[8; 32]));
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[9; 32]));
    }

    #[test]
    fn bridge_transaction_manager_runtime_insert_validated_rejects_metadata_mismatch() {
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        let mut input = runtime_queue_input(10, true);
        input.gas = 99_999;

        let err = match runtime.transaction_manager_runtime_insert_validated_transaction(
            validated_insert_sidecar_fact(10, true, 0, 100, false),
            input,
        ) {
            Ok(_) => panic!("runtime validated insert should reject mismatched gas"),
            Err(err) => err,
        };
        assert!(err
            .to_string()
            .contains("TM_RUNTIME_VALIDATED_INSERT_GAS_MISMATCH"));
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[10; 32]));
    }

    #[test]
    fn bridge_transaction_manager_runtime_queue_cleanup_returns_explicit_hash_groups() {
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 32 });
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input(1, true))
            .expect("proposable insert should succeed");
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input(2, false))
            .expect("non-proposable insert should succeed");
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input(3, false))
            .expect("non-proposable insert should succeed");

        let cleanup = runtime.transaction_manager_runtime_queue_cleanup(
            true,
            20,
            vec![BridgeTransactionQueueAccountNonceFact {
                sender: [9; 20],
                account_found: true,
                account_nonce: U256::from(2_u64).to_big_endian(),
            }],
        );

        assert_eq!(cleanup.non_proposable_expired.removed_count, 2);
        assert_eq!(
            cleanup.non_proposable_expired.removed_hashes[0].hash,
            [2; 32]
        );
        assert_eq!(
            cleanup.non_proposable_expired.removed_hashes[1].hash,
            [3; 32]
        );
        assert_eq!(cleanup.finalized_account_purged.removed_count, 1);
        assert_eq!(
            cleanup.finalized_account_purged.removed_hashes[0].hash,
            [1; 32]
        );
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[1; 32]));
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[2; 32]));
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[3; 32]));
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
    fn bridge_transaction_manager_verify_not_finalized_with_runtime_and_final_chain_gates_lookup() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_verify_not_finalized_fc");
        let storage = crate::storage::create_storage(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
        )
        .expect("storage should initialize");
        let sender = [5; 20];
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
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("final chain should initialize");
        storage
            .save_transaction_location(&[2u8; 32], 7, 0, false)
            .expect("finalized hash should be persisted");
        let runtime = create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        let out = transaction_manager_verify_not_finalized_with_runtime_and_final_chain(
            &runtime,
            &storage,
            &final_chain,
            vec![
                verify_not_finalized_runtime_fact(0, 1, 10, sender),
                verify_not_finalized_runtime_fact(1, 2, 0, sender),
            ],
        )
        .expect("final-chain gated verify should execute");
        assert!(out.is_finalized);
        assert_eq!(out.input_index, 1);
        assert_eq!(out.hash, [2; 32]);
        assert_eq!(out.source, TM_VERIFY_NOT_FINALIZED_SOURCE_STORAGE);
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
            .transaction_manager_runtime_pack_begin(63_000, 21_000, 7, 0, 10)
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
            .transaction_manager_runtime_pack_begin(63_000, 21_000, 7, 0, 10)
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
            .transaction_manager_runtime_pack_begin(21_000, 21_000, 7, 0, 10)
            .expect("completed step session should be cleared");
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
            .transaction_manager_runtime_pack_begin(63_000, 21_000, 7, 25_000, 10)
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
            .transaction_manager_runtime_pack_begin(63_000, 21_000, 7, 0, 10)
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
        assert_eq!(
            runtime.transaction_manager_runtime_gas_estimation_cache_size(),
            1
        );

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
