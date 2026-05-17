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
    NonFinalizedTransactionPayload, TransactionManagerFilterAction,
    TransactionManagerFinalizedFilterFact, TransactionManagerGasEstimationFact,
    TransactionManagerGasEstimationPlan, TransactionManagerGasEstimationResult,
    TransactionManagerInsertTransactionFact, TransactionManagerInsertTransactionOutcome,
    TransactionManagerLifecycleNotice, TransactionManagerLifecycleReport,
    TransactionManagerRecoveryEntry, TransactionManagerRuntimeAdmissionOutcome,
    TransactionManagerRuntimeQueueCleanupPlan, TransactionManagerRuntimeValidatedInsertOutcome,
    TransactionManagerSidecarInsertInput, TransactionManagerSidecarKnownFact,
    TransactionManagerSidecarLookup, TransactionManagerSidecarLookupPlan,
    TransactionManagerSidecarLookupRequest, TransactionManagerSidecarRecoveryInsertInput,
    TransactionManagerSidecarTransitionInput, TransactionManagerStoredTransactionLookup,
    TransactionManagerStoredTransactionRequest, TransactionManagerValidatedInsertFact,
    TransactionManagerValidatedInsertPlan, TransactionManagerValidatedInsertRuntimeFact,
    TransactionManagerValidatedInsertSidecarFact, TransactionManagerVerifyNotFinalizedFact,
    TransactionManagerVerifyNotFinalizedOutcome, TransactionManagerVerifyNotFinalizedRuntimeFact,
    TransactionManagerVerifyNotFinalizedSidecarFact, TransactionManagerVerifyTransactionFact,
    TransactionManagerVerifyTransactionOutcome, TransactionPackCandidateDecision,
    TransactionPackCandidateInput, TransactionPackEstimateInput, TransactionPackEstimateOutcome,
    TransactionPackSelectedTransaction, TransactionPackSessionCandidate,
    TransactionPackSessionEstimateInput, TransactionPackSessionOutcome, TransactionPackSessionStep,
    TransactionQueueAccountNonceFact as BridgeTransactionQueueAccountNonceFact,
    TransactionQueueAddress, TransactionQueueConfig, TransactionQueueDemotePlan,
    TransactionQueueHash, TransactionQueueInsertInput, TransactionQueueInsertOutcome,
    TransactionQueuePurgePlan, TransactionQueueStoredTransaction, TransactionQueueTransactionGroup,
};
use crate::ffi::{
    BridgeFinalChain, BridgeStorage, BridgeTransactionManagerAdmissionExecution,
    BridgeTransactionManagerRuntime, BridgeTransactionPackPlanner,
    TransactionManagerRuntimePackSession,
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
    TransactionQueue, TransactionQueueAccountNonceFact, TransactionQueueDemoteOutcome,
    TransactionQueueDemoteStatus, TransactionQueueEntry, TransactionQueueInsertStatus,
    TransactionQueuePurgeOutcome,
};
use rustaxa_types::LegacyTransactionEnvelope;
use std::time::{Duration, Instant};

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
const TRANSACTION_QUEUE_DEMOTE_STATUS_NOT_FOUND: u8 = 0;
const TRANSACTION_QUEUE_DEMOTE_STATUS_ALREADY_NON_PROPOSABLE: u8 = 1;
const TRANSACTION_QUEUE_DEMOTE_STATUS_DEMOTED: u8 = 2;
/// Notices are compact, stable discriminators for generic CXX lifecycle replay.
const TM_LIFECYCLE_NOTICE_DAG_ACCEPTED: u8 = 0;
const TM_LIFECYCLE_NOTICE_DAG_QUEUE_ERASED: u8 = 1;
const TM_LIFECYCLE_NOTICE_FINALIZED_REMOVED_NON_FINALIZED: u8 = 2;
const TM_LIFECYCLE_NOTICE_FINALIZED_QUEUE_ERASED: u8 = 3;
const TM_LIFECYCLE_NOTICE_ADMISSION_INSERTED: u8 = 4;
const TM_LIFECYCLE_NOTICE_ADMISSION_TRANSACTION_ADDED: u8 = 5;
const TM_LIFECYCLE_NOTICE_ADMISSION_STATUS_ACCEPTED: u8 = 6;
const TM_LIFECYCLE_NOTICE_ADMISSION_STATUS_ALREADY_KNOWN: u8 = 7;
const TM_LIFECYCLE_NOTICE_ADMISSION_STATUS_ALREADY_FINALIZED: u8 = 8;
const TM_LIFECYCLE_NOTICE_ADMISSION_STATUS_CANNOT_INSERT: u8 = 9;
const TM_LIFECYCLE_NOTICE_ADMISSION_FINALIZED_LOOKUP_REQUIRED: u8 = 10;
const TM_LIFECYCLE_NOTICE_ADMISSION_DEMOTED_HASH: u8 = 11;
const TM_LIFECYCLE_NOTICE_ADMISSION_OVERFLOW_REMOVED_HASH: u8 = 12;
const TM_LIFECYCLE_NOTICE_RECOVERY_INSERTED: u8 = 13;
const TRANSACTION_QUEUE_DROP_WINDOW: Duration = Duration::from_secs(600);

fn lifecycle_notice(
    kind: u8,
    input_index: u64,
    hash: [u8; 32],
) -> TransactionManagerLifecycleNotice {
    TransactionManagerLifecycleNotice {
        kind,
        input_index,
        hash,
    }
}

fn dag_save_notices(outcome: &DagTransactionSaveOutcome) -> Vec<TransactionManagerLifecycleNotice> {
    let mut notices = Vec::with_capacity(outcome.accepted.len() * 2);

    for entry in &outcome.accepted {
        notices.push(lifecycle_notice(
            TM_LIFECYCLE_NOTICE_DAG_ACCEPTED,
            entry.input_index,
            entry.hash,
        ));
        if entry.erased_from_queue {
            notices.push(lifecycle_notice(
                TM_LIFECYCLE_NOTICE_DAG_QUEUE_ERASED,
                entry.input_index,
                entry.hash,
            ));
        }
    }

    notices
}

fn finalized_status_notices(
    outcome: &FinalizedTransactionStatusPlan,
) -> Vec<TransactionManagerLifecycleNotice> {
    let mut notices = Vec::with_capacity(outcome.accepted.len() * 2);

    for action in &outcome.accepted {
        if action.removed_non_finalized {
            notices.push(lifecycle_notice(
                TM_LIFECYCLE_NOTICE_FINALIZED_REMOVED_NON_FINALIZED,
                action.input_index,
                action.hash,
            ));
        }
        if action.erase_from_queue && action.erased_from_queue {
            notices.push(lifecycle_notice(
                TM_LIFECYCLE_NOTICE_FINALIZED_QUEUE_ERASED,
                action.input_index,
                action.hash,
            ));
        }
    }

    notices
}

fn admission_status_notice(kind: u8, input_hash: [u8; 32]) -> TransactionManagerLifecycleNotice {
    lifecycle_notice(kind, 0, input_hash)
}

fn admission_notices(
    outcome: &TransactionManagerRuntimeAdmissionOutcome,
    input_hash: [u8; 32],
) -> Vec<TransactionManagerLifecycleNotice> {
    let mut notices = Vec::new();

    notices.push(admission_status_notice(
        match outcome.insert_status {
            x if x == TM_INSERT_TRANSACTION_STATUS_ACCEPTED => {
                TM_LIFECYCLE_NOTICE_ADMISSION_STATUS_ACCEPTED
            }
            x if x == TM_INSERT_TRANSACTION_STATUS_ALREADY_KNOWN => {
                TM_LIFECYCLE_NOTICE_ADMISSION_STATUS_ALREADY_KNOWN
            }
            x if x == TM_INSERT_TRANSACTION_STATUS_ALREADY_FINALIZED => {
                TM_LIFECYCLE_NOTICE_ADMISSION_STATUS_ALREADY_FINALIZED
            }
            _ => TM_LIFECYCLE_NOTICE_ADMISSION_STATUS_CANNOT_INSERT,
        },
        input_hash,
    ));

    if outcome.inserted_hash_found {
        notices.push(admission_status_notice(
            TM_LIFECYCLE_NOTICE_ADMISSION_INSERTED,
            outcome.inserted_hash,
        ));

        if outcome.emit_transaction_added {
            notices.push(admission_status_notice(
                TM_LIFECYCLE_NOTICE_ADMISSION_TRANSACTION_ADDED,
                outcome.inserted_hash,
            ));
        }
    }

    for entry in outcome.demoted_hashes.iter() {
        notices.push(admission_status_notice(
            TM_LIFECYCLE_NOTICE_ADMISSION_DEMOTED_HASH,
            entry.hash,
        ));
    }

    for entry in outcome.overflow_removed_hashes.iter() {
        notices.push(admission_status_notice(
            TM_LIFECYCLE_NOTICE_ADMISSION_OVERFLOW_REMOVED_HASH,
            entry.hash,
        ));
    }

    if outcome.requires_finalized_lookup {
        notices.push(admission_status_notice(
            TM_LIFECYCLE_NOTICE_ADMISSION_FINALIZED_LOOKUP_REQUIRED,
            input_hash,
        ));
    }

    notices
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
            tx_rlp: entry.rlp,
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
            tx_rlp: Vec::new(),
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
        tx_rlp: Vec::new(),
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

fn runtime_pack_session_step(
    session: &mut TransactionManagerRuntimePackSession,
) -> Result<TransactionPackSessionStep> {
    if let Some(candidate) = runtime_pack_next_estimable_entry(session)? {
        return Ok(TransactionPackSessionStep {
            request_estimate: true,
            candidate: transaction_pack_candidate_from_entry(Some(candidate))?,
            selected_transactions: Vec::new(),
            demoted_hashes: Vec::new(),
            stopped: session.stopped,
        });
    }

    Ok(TransactionPackSessionStep {
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
    })
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

fn runtime_empty_queue_entry() -> TransactionQueueEntry {
    TransactionQueueEntry {
        hash: H256::zero(),
        sender: H160::zero(),
        nonce: U256::zero(),
        gas_price: U256::zero(),
        gas: 0,
        data_size: 0,
        rlp: Vec::new(),
        last_block_number: 0,
    }
}

fn runtime_queue_demote_plan_from_consensus(
    hash: H256,
    outcome: TransactionQueueDemoteOutcome,
) -> TransactionQueueDemotePlan {
    let hash_found = outcome.entry.is_some();
    let entry = outcome.entry.unwrap_or_else(runtime_empty_queue_entry);
    TransactionQueueDemotePlan {
        status: match outcome.status {
            TransactionQueueDemoteStatus::NotFound => TRANSACTION_QUEUE_DEMOTE_STATUS_NOT_FOUND,
            TransactionQueueDemoteStatus::AlreadyNonProposable => {
                TRANSACTION_QUEUE_DEMOTE_STATUS_ALREADY_NON_PROPOSABLE
            }
            TransactionQueueDemoteStatus::Demoted => TRANSACTION_QUEUE_DEMOTE_STATUS_DEMOTED,
        },
        hash: hash.0,
        hash_found,
        sender: entry.sender.0,
        nonce: entry.nonce.to_big_endian(),
        gas_price: entry.gas_price.to_big_endian(),
        gas: entry.gas,
        data_size: entry.data_size as usize,
        last_block_number: entry.last_block_number,
        proposable_before: matches!(outcome.status, TransactionQueueDemoteStatus::Demoted),
    }
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

/// Executes runtime DAG persistence using FinalChain sender nonces and returns a
/// replay-safe lifecycle report for shim follow-up behavior.
pub fn save_transactions_from_dag_block_report_with_runtime_and_final_chain(
    runtime: &mut BridgeTransactionManagerRuntime,
    storage: &BridgeStorage,
    final_chain: &BridgeFinalChain,
    facts: Vec<DagTransactionSaveRuntimeFact>,
) -> Result<TransactionManagerLifecycleReport> {
    let outcome = save_transactions_from_dag_block_with_runtime_and_final_chain(
        runtime,
        storage,
        final_chain,
        facts,
    )?;
    Ok(TransactionManagerLifecycleReport {
        notices: dag_save_notices(&outcome),
        transaction_count: outcome.target_transaction_count,
        purge_transaction_queue: false,
    })
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

/// Applies finalized-transaction status changes through runtime and returns replay
/// notices for shim-owned side effects.
pub fn update_finalized_transactions_status_report_with_runtime(
    runtime: &mut BridgeTransactionManagerRuntime,
    storage: &BridgeStorage,
    period: u64,
    retention_window: u64,
    facts: Vec<FinalizedTransactionStatusSidecarFact>,
) -> Result<TransactionManagerLifecycleReport> {
    let outcome = update_finalized_transactions_status_with_runtime(
        runtime,
        storage,
        period,
        retention_window,
        facts,
    )?;
    Ok(TransactionManagerLifecycleReport {
        notices: finalized_status_notices(&outcome),
        transaction_count: outcome.target_transaction_count,
        purge_transaction_queue: outcome.purge_transaction_queue,
    })
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

/// Rebuilds runtime sidecars from recovery inputs and returns shim-side notices.
///
/// Recovery first validates persisted rows through the existing recovery-input
/// pipeline so malformed rows never reach live runtime sidecar state.
pub fn transaction_manager_recover_nonfinalized_report_with_runtime(
    runtime: &mut BridgeTransactionManagerRuntime,
    storage: &BridgeStorage,
) -> Result<TransactionManagerLifecycleReport> {
    let entries = transaction_manager_load_nonfinalized_recovery_inputs(storage)?;
    let notices = entries
        .iter()
        .map(|entry| lifecycle_notice(TM_LIFECYCLE_NOTICE_RECOVERY_INSERTED, 0, entry.hash))
        .collect();
    runtime.transaction_manager_runtime_insert_recovery_entries(entries)?;

    Ok(TransactionManagerLifecycleReport {
        notices,
        transaction_count: runtime.transaction_manager_runtime_transaction_count(),
        purge_transaction_queue: false,
    })
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
            candidates,
            next_index: 0,
            current: None,
            selected: Vec::new(),
            demoted_hashes: Vec::new(),
            stopped: false,
        });
        Ok(())
    }

    /// Returns the next queue candidate that needs C++ gas estimation.
    pub fn transaction_manager_runtime_pack_next_candidate(
        &mut self,
    ) -> Result<TransactionPackSessionCandidate> {
        let session = self
            .transaction_pack_session
            .as_mut()
            .context("TM_RUNTIME_PACK_SESSION_NOT_ACTIVE")?;
        runtime_pack_next_estimable_entry(session).and_then(transaction_pack_candidate_from_entry)
    }

    /// Requests the next packed transaction candidate or final session outcome.
    pub fn transaction_manager_runtime_pack_request_next(
        &mut self,
    ) -> Result<TransactionPackSessionStep> {
        let mut session = self
            .transaction_pack_session
            .take()
            .context("TM_RUNTIME_PACK_SESSION_NOT_ACTIVE")?;
        let step = match runtime_pack_session_step(&mut session) {
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

    /// Records one C++ gas estimate and applies any Rust-owned queue demotion.
    pub fn transaction_manager_runtime_pack_record_estimate(
        &mut self,
        input: TransactionPackSessionEstimateInput,
    ) -> Result<TransactionPackEstimateOutcome> {
        let mut session = self
            .transaction_pack_session
            .take()
            .context("TM_RUNTIME_PACK_SESSION_NOT_ACTIVE")?;

        let outcome = match self
            .transaction_manager_runtime_pack_record_estimate_inner(&mut session, input)
        {
            Ok(outcome) => outcome,
            Err(err) => {
                self.transaction_pack_session = Some(session);
                return Err(err).context("TM_RUNTIME_PACK_RECORD_ESTIMATE");
            }
        };

        self.transaction_pack_session = Some(session);
        Ok(outcome)
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

        let out = match runtime_pack_session_step(&mut session) {
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

    /// Finalizes, clears, and returns the active runtime packing session outcome.
    pub fn transaction_manager_runtime_pack_finalize(
        &mut self,
    ) -> Result<TransactionPackSessionOutcome> {
        let session = self
            .transaction_pack_session
            .take()
            .context("TM_RUNTIME_PACK_SESSION_NOT_ACTIVE")?;
        Ok(TransactionPackSessionOutcome {
            selected_transactions: session
                .selected
                .into_iter()
                .map(|(entry, gas_used)| TransactionPackSelectedTransaction {
                    hash: entry.hash.0,
                    gas_used,
                    tx_rlp: entry.rlp,
                })
                .collect(),
            demoted_hashes: runtime_hashes_to_bridge(session.demoted_hashes),
            stopped: session.stopped,
        })
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

        if let Some(result_rlp) = self
            .sidecar
            .gas_estimation_cache_get(hash, fact.proposal_period)
            .context("TM_RUNTIME_GAS_ESTIMATION_CACHE_GET")?
        {
            return Ok(TransactionManagerGasEstimationPlan {
                use_declared_gas: false,
                cache_hit: true,
                requires_evm_call: false,
                gas_used: 0,
                result_rlp,
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

    /// Finishes public `insertTransaction` status selection after C++ supplies any requested finalized-location fact.
    ///
    /// This is intentionally read-only. Rust owns the public status mapping,
    /// while C++ remains the executor for storage lookup because transaction
    /// location storage has not moved fully behind this runtime boundary.
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

    /// Executes admission and returns replay notices for shim-owned lifecycle hooks.
    pub fn transaction_manager_runtime_execute_transaction_admission_report(
        &mut self,
        fact: TransactionManagerValidatedInsertSidecarFact,
        input: TransactionQueueInsertInput,
        has_finalized_period: bool,
        finalized_period: u64,
    ) -> Result<TransactionManagerLifecycleReport> {
        let input_hash = fact.tx_hash;
        let outcome = self.transaction_manager_runtime_execute_transaction_admission(
            fact,
            input,
            has_finalized_period,
            finalized_period,
        )?;
        Ok(TransactionManagerLifecycleReport {
            notices: admission_notices(&outcome, input_hash),
            transaction_count: self.transaction_manager_runtime_transaction_count(),
            purge_transaction_queue: false,
        })
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

    /// Executes FinalChain-backed admission and returns replay notices for shim-owned lifecycle hooks.
    pub fn transaction_manager_runtime_execute_transaction_admission_with_final_chain_report(
        &mut self,
        final_chain: &BridgeFinalChain,
        fact: TransactionManagerValidatedInsertRuntimeFact,
        input: TransactionQueueInsertInput,
    ) -> Result<TransactionManagerLifecycleReport> {
        let input_hash = fact.tx_hash;
        let outcome = self
            .transaction_manager_runtime_execute_transaction_admission_with_final_chain(
                final_chain,
                fact,
                input,
            )?;
        Ok(TransactionManagerLifecycleReport {
            notices: admission_notices(&outcome, input_hash),
            transaction_count: self.transaction_manager_runtime_transaction_count(),
            purge_transaction_queue: false,
        })
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

    /// Returns proposer accounts that C++ should query from FinalChain for purge facts.
    pub fn transaction_manager_runtime_queue_proposable_accounts(
        &self,
    ) -> Vec<TransactionQueueAddress> {
        self.queue
            .proposable_accounts()
            .into_iter()
            .map(|address| TransactionQueueAddress { address: address.0 })
            .collect()
    }

    /// Removes queued transactions for account nonce facts supplied by C++ FinalChain reads.
    pub fn transaction_manager_runtime_queue_purge_accounts_plan(
        &mut self,
        facts: Vec<BridgeTransactionQueueAccountNonceFact>,
    ) -> TransactionQueuePurgePlan {
        let consensus_facts = runtime_queue_account_nonce_facts_from_bridge(facts);
        runtime_queue_purge_plan_from_consensus(self.queue.purge_accounts_plan(&consensus_facts))
    }

    /// Applies Rust-owned queue cleanup for finalized block height and/or FinalChain account facts.
    ///
    /// C++ supplies account nonce facts because FinalChain account reads remain
    /// in the shim. Rust owns all queue mutation and returns explicit removed
    /// hash groups for C++ logging or future side-effect execution.
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

    /// Demotes one queued transaction to non-proposable metadata.
    pub fn transaction_manager_runtime_queue_demote_to_non_proposable(
        &mut self,
        hash: &[u8; 32],
        last_block_number: u64,
    ) -> TransactionQueueDemotePlan {
        let parsed_hash = H256::from(*hash);
        runtime_queue_demote_plan_from_consensus(
            parsed_hash,
            self.queue.demote(parsed_hash, last_block_number),
        )
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

/// Creates a planner for one `TransactionManager::packTrxs` invocation.
pub fn create_transaction_pack_planner(
    weight_limit: u64,
    min_transaction_gas: u64,
) -> Result<Box<BridgeTransactionPackPlanner>> {
    Ok(Box::new(BridgeTransactionPackPlanner(
        TransactionPackingPlanner::new(weight_limit, min_transaction_gas)?,
    )))
}

impl BridgeTransactionPackPlanner {
    /// Returns the maximum number of ordered queue candidates C++ should snapshot.
    pub fn transaction_pack_max_candidate_count(&self) -> u64 {
        self.0.max_candidate_count()
    }

    /// Decides whether C++ should run a live gas estimate for this candidate.
    pub fn transaction_pack_consider_candidate(
        &self,
        input: TransactionPackCandidateInput,
    ) -> Result<TransactionPackCandidateDecision> {
        let decision = self.0.consider_candidate(TransactionPackCandidate {
            hash: H256::from(input.hash),
            declared_gas: input.declared_gas,
        })?;
        Ok(TransactionPackCandidateDecision {
            should_estimate: decision.should_estimate,
        })
    }

    /// Records a C++ gas estimate and returns the live-state action C++ must apply.
    pub fn transaction_pack_record_estimate(
        &mut self,
        input: TransactionPackEstimateInput,
    ) -> Result<TransactionPackEstimateOutcome> {
        let outcome = self.0.record_estimate(TransactionPackEstimate {
            hash: H256::from(input.hash),
            gas_used: input.gas_used,
        })?;
        Ok(TransactionPackEstimateOutcome {
            hash: outcome.hash.0,
            selected: outcome.selected,
            demote_to_non_proposable: outcome.demote_to_non_proposable,
            stop: outcome.stop,
            gas_used: outcome.gas_used,
        })
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

    #[test]
    fn bridge_planner_round_trips_estimation_decisions() {
        let mut planner = create_transaction_pack_planner(63_000, 21_000).unwrap();
        assert_eq!(planner.transaction_pack_max_candidate_count(), 3);

        let decision = planner
            .transaction_pack_consider_candidate(TransactionPackCandidateInput {
                hash: [1; 32],
                declared_gas: 42_000,
            })
            .unwrap();
        assert!(decision.should_estimate);

        let outcome = planner
            .transaction_pack_record_estimate(TransactionPackEstimateInput {
                hash: [1; 32],
                gas_used: 42_000,
            })
            .unwrap();
        assert!(outcome.selected);
        assert_eq!(outcome.hash, [1; 32]);
        assert!(outcome.stop);
    }

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
    fn bridge_save_transactions_from_dag_block_report_with_runtime_and_final_chain_emits_notices() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_runtime_admission_fc_report");
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
        runtime
            .transaction_manager_runtime_queue_insert(runtime_queue_input(1, true))
            .expect("queue seed should succeed");

        let report = save_transactions_from_dag_block_report_with_runtime_and_final_chain(
            &mut runtime,
            &storage,
            &final_chain,
            vec![dag_tx_runtime_fact(0, 1, 1, sender, 0x33)],
        )
        .expect("runtime final-chain DAG report should execute");

        assert_eq!(report.transaction_count, 8);
        assert!(!report.purge_transaction_queue);
        assert_eq!(report.notices.len(), 2);
        assert_eq!(report.notices[0].kind, TM_LIFECYCLE_NOTICE_DAG_ACCEPTED);
        assert_eq!(report.notices[0].input_index, 0);
        assert_eq!(report.notices[0].hash, [1; 32]);
        assert_eq!(report.notices[1].kind, TM_LIFECYCLE_NOTICE_DAG_QUEUE_ERASED);
        assert_eq!(report.notices[1].input_index, 0);
        assert_eq!(report.notices[1].hash, [1; 32]);
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
    fn bridge_update_finalized_transactions_status_report_with_runtime_emits_notices() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tm_update_finalized_status_report");
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

        let report = update_finalized_transactions_status_report_with_runtime(
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
        .expect("runtime finalized status report should execute");

        assert_eq!(report.transaction_count, 7);
        assert!(report.purge_transaction_queue == false);
        assert_eq!(report.notices.len(), 2);
        assert_eq!(
            report.notices[0].kind,
            TM_LIFECYCLE_NOTICE_FINALIZED_REMOVED_NON_FINALIZED
        );
        assert_eq!(
            report.notices[1].kind,
            TM_LIFECYCLE_NOTICE_FINALIZED_QUEUE_ERASED
        );
        assert_eq!(report.notices[0].hash, [1; 32]);
        assert!(!runtime.transaction_manager_runtime_queue_contains(&[1; 32]));
        assert!(runtime.transaction_manager_runtime_contains_recently_finalized(&[1; 32]));

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
    fn bridge_transaction_manager_recover_nonfinalized_report_inserts_survivors() {
        let temp_dir =
            unique_temp_dir("rustaxa_bridge_transaction_manager_recover_nonfinalized_report");
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

        let report =
            transaction_manager_recover_nonfinalized_report_with_runtime(&mut runtime, &storage)
                .expect("runtime recovery report should execute");

        assert_eq!(report.transaction_count, 4);
        assert!(!report.purge_transaction_queue);
        assert_eq!(report.notices.len(), 1);
        assert_eq!(
            report.notices[0].kind,
            TM_LIFECYCLE_NOTICE_RECOVERY_INSERTED
        );
        assert_eq!(report.notices[0].hash, live_hash);
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
    fn bridge_transaction_manager_runtime_admission_report_includes_status_and_added() {
        let mut runtime =
            create_transaction_manager_runtime(0, TransactionQueueConfig { max_size: 8 });
        let report = runtime
            .transaction_manager_runtime_execute_transaction_admission_report(
                validated_insert_sidecar_fact(14, true, 0, 100, false),
                runtime_queue_input(14, false),
                false,
                0,
            )
            .expect("runtime admission report should execute");

        assert_eq!(report.transaction_count, 0);
        assert!(!report.purge_transaction_queue);
        assert_eq!(report.notices.len(), 3);
        assert_eq!(
            report.notices[0].kind,
            TM_LIFECYCLE_NOTICE_ADMISSION_STATUS_ACCEPTED
        );
        assert_eq!(report.notices[0].hash, [14; 32]);
        assert_eq!(
            report.notices[1].kind,
            TM_LIFECYCLE_NOTICE_ADMISSION_INSERTED
        );
        assert_eq!(report.notices[1].hash, [14; 32]);
        assert_eq!(
            report.notices[2].kind,
            TM_LIFECYCLE_NOTICE_ADMISSION_TRANSACTION_ADDED
        );
        assert_eq!(report.notices[2].hash, [14; 32]);
        assert!(runtime.transaction_manager_runtime_queue_contains(&[14; 32]));
    }

    #[test]
    fn bridge_transaction_manager_runtime_admission_with_final_chain_report_includes_status() {
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
            .transaction_manager_runtime_execute_transaction_admission_with_final_chain_report(
                &final_chain,
                validated_insert_runtime_fact(15, sender, false),
                runtime_queue_input(15, false),
            )
            .expect("runtime admission final-chain report should execute");

        assert_eq!(report.transaction_count, 0);
        assert_eq!(report.notices.len(), 3);
        assert_eq!(
            report.notices[0].kind,
            TM_LIFECYCLE_NOTICE_ADMISSION_STATUS_ACCEPTED
        );
        assert_eq!(report.notices[0].hash, [15; 32]);
        assert_eq!(
            report.notices[1].kind,
            TM_LIFECYCLE_NOTICE_ADMISSION_INSERTED
        );
        assert_eq!(report.notices[1].hash, [15; 32]);
        assert_eq!(
            report.notices[2].kind,
            TM_LIFECYCLE_NOTICE_ADMISSION_TRANSACTION_ADDED
        );
        assert_eq!(report.notices[2].hash, [15; 32]);
        assert!(runtime.transaction_manager_runtime_queue_contains(&[15; 32]));
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
            .transaction_manager_runtime_pack_begin(63_000, 21_000)
            .expect("pack session should begin");

        let candidate = runtime
            .transaction_manager_runtime_pack_next_candidate()
            .expect("candidate decision should succeed");
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

        let outcome = runtime
            .transaction_manager_runtime_pack_record_estimate(TransactionPackSessionEstimateInput {
                hash: envelope.hash.0,
                gas_used: 42_000,
                last_block_number: 10,
            })
            .expect("record estimate should succeed");
        assert!(outcome.selected);
        assert!(outcome.stop);
        assert!(!outcome.demote_to_non_proposable);

        let final_outcome = runtime
            .transaction_manager_runtime_pack_finalize()
            .expect("pack session should finalize");
        assert_eq!(final_outcome.selected_transactions.len(), 1);
        assert_eq!(final_outcome.selected_transactions[0].hash, envelope.hash.0);
        assert_eq!(final_outcome.selected_transactions[0].gas_used, 42_000);
        assert!(runtime
            .transaction_manager_runtime_pack_next_candidate()
            .is_err());
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
            .transaction_manager_runtime_pack_begin(63_000, 21_000)
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
                },
            )
            .expect("loop should finalize after last candidate");
        assert!(!final_step.request_estimate);
        assert_eq!(final_step.selected_transactions.len(), 1);
        assert_eq!(final_step.selected_transactions[0].hash, first_hash);
        assert_eq!(final_step.selected_transactions[0].gas_used, 30_000);
        assert_eq!(final_step.demoted_hashes.len(), 1);
        assert_eq!(final_step.demoted_hashes[0].hash, second_hash);

        assert!(runtime.transaction_manager_runtime_pack_finalize().is_err());
        runtime
            .transaction_manager_runtime_pack_begin(21_000, 21_000)
            .expect("completed step session should be cleared");
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
