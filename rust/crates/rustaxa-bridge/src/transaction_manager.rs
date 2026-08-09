//! CXX bridge wrappers for Rust `TransactionManager` decision boundaries.
//!
//! The bridge exposes:
//! - a short-lived planner used while one DAG proposal is being packed
//! - owned carrier conversion for transaction commands and reports
//!
//! C++ supplies transaction metadata, canonical RLP payloads, and retained
//! FinalChain/EVM facts. Native Rust services own transaction state, locking,
//! deterministic planning, storage mutation, count authority, admission, and
//! sidecar/queue publication. This module owns no production runtime or guard.

use crate::ffi::rustaxa_ffi::{
    GasPricerConfig, TransactionManagerAdmissionCommandReport, TransactionManagerAdmissionResult,
    TransactionManagerAdmissionShellIntent, TransactionManagerDagSaveCommandReport,
    TransactionManagerFinalChainAdmissionFact, TransactionManagerGasEstimationFact,
    TransactionManagerGasEstimationPlan, TransactionManagerPublicAdmissionCommandReport,
    TransactionManagerPublicInsertResult, TransactionManagerTransactionView,
    TransactionManagerTransactionViewPlan, TransactionManagerTransactionViewRequest,
    TransactionManagerValidatedInsertRuntimeFact, TransactionManagerVerifyTransactionFact,
    TransactionManagerVerifyTransactionOutcome,
    TransactionQueueAccountNonceFact as BridgeTransactionQueueAccountNonceFact,
    TransactionQueueHash, TransactionQueueInsertInput, TransactionQueueStoredTransaction,
    TransactionQueueTransactionGroup,
};
use anyhow::Result;
use ethereum_types::{H160, H256, U256};
use rustaxa_consensus::gas_pricer::GasPricerConfig as DomainGasPricerConfig;
use rustaxa_consensus::transaction_manager::{
    plan_verify_transaction, TransactionManagerInsertTransactionStatus,
    TransactionManagerVerifyTransactionFact as ConsensusTransactionManagerVerifyTransactionFact,
    TransactionManagerVerifyTransactionStatus,
};
use rustaxa_consensus::transaction_queue::{TransactionQueueEntry, TransactionQueueInsertStatus};
use rustaxa_consensus::transaction_service::{
    DagTransactionSaveOutcome, TransactionServiceAccountNonceFact,
    TransactionServiceAdmissionReport, TransactionServiceFinalChainAdmissionFact,
    TransactionServiceGasEstimationPlan, TransactionServiceGasEstimationRequest,
    TransactionServicePublicAdmissionReport, TransactionServiceTransactionView,
    TransactionServiceTransactionViewPlan, TransactionServiceTransactionViewRequest,
    TransactionServiceValidatedAdmissionFact,
};
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

pub(crate) fn bridge_to_service_validated_admission_fact(
    fact: TransactionManagerValidatedInsertRuntimeFact,
) -> TransactionServiceValidatedAdmissionFact {
    TransactionServiceValidatedAdmissionFact {
        tx_hash: H256::from(fact.tx_hash),
        transaction_nonce: U256::from_big_endian(&fact.transaction_nonce),
        transaction_cost: U256::from_big_endian(&fact.transaction_cost),
        gas_limit: fact.gas_limit,
        proposal_dag_gas_limit: fact.propose_dag_gas_limit,
        insert_non_proposable: fact.insert_non_proposable,
    }
}

pub(crate) fn bridge_to_service_final_chain_admission_fact(
    fact: TransactionManagerFinalChainAdmissionFact,
) -> TransactionServiceFinalChainAdmissionFact {
    TransactionServiceFinalChainAdmissionFact {
        account_found: fact.account_found,
        account_nonce: U256::from_big_endian(&fact.account_nonce),
        account_balance: U256::from_big_endian(&fact.account_balance),
        finalized_period: fact.finalized_period_known.then_some(fact.finalized_period),
    }
}

pub(crate) fn bridge_to_service_queue_entry(
    input: &TransactionQueueInsertInput,
) -> TransactionQueueEntry {
    runtime_queue_entry_from_insert_input(input)
}

pub(crate) fn service_admission_to_bridge(
    report: TransactionServiceAdmissionReport,
) -> TransactionManagerAdmissionCommandReport {
    let inserted_hash = report.inserted_hash.unwrap_or_default().0;
    let mut shell_intents = Vec::new();
    if report.inserted_hash.is_some() {
        shell_intents.push(TransactionManagerAdmissionShellIntent {
            kind: TM_ADMISSION_SHELL_INTENT_LOG_INSERTED,
            hash: inserted_hash,
        });
    }
    if report.emit_transaction_added && report.inserted_hash.is_some() {
        shell_intents.push(TransactionManagerAdmissionShellIntent {
            kind: TM_ADMISSION_SHELL_INTENT_EMIT_TRANSACTION_ADDED,
            hash: inserted_hash,
        });
    }
    TransactionManagerAdmissionCommandReport {
        inserted_hash_found: report.inserted_hash.is_some(),
        inserted_hash,
        transaction_added_hash_found: report.emit_transaction_added
            && report.inserted_hash.is_some(),
        transaction_added_hash: inserted_hash,
        shell_intents,
        admission: TransactionManagerAdmissionResult {
            present: true,
            insert_status: report.insert_status as u8,
            transaction_status: queue_status_to_ffi(report.transaction_status),
            finalized_period_known: report.finalized_period.is_some(),
            finalized_period: report.finalized_period.unwrap_or_default(),
            requires_finalized_lookup: false,
        },
    }
}

pub(crate) fn service_public_admission_to_bridge(
    report: TransactionServicePublicAdmissionReport,
) -> TransactionManagerPublicAdmissionCommandReport {
    let admission = report.admission.map(service_admission_to_bridge).unwrap_or(
        TransactionManagerAdmissionCommandReport {
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
        },
    );
    TransactionManagerPublicAdmissionCommandReport {
        verification_status: report.verification_status as u8,
        verification_chain_id: report.verification_chain_id,
        verification_expected_chain_id: report.verification_expected_chain_id,
        public_result: TransactionManagerPublicInsertResult {
            accepted: report.public_result.accepted,
            message: report.public_result.message,
        },
        admission,
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
const TM_ADMISSION_SHELL_INTENT_LOG_INSERTED: u8 = 1;
const TM_ADMISSION_SHELL_INTENT_EMIT_TRANSACTION_ADDED: u8 = 2;
fn hash_command(hash: [u8; 32]) -> TransactionQueueHash {
    TransactionQueueHash { hash }
}

pub(crate) fn dag_save_command_report(
    outcome: &DagTransactionSaveOutcome,
) -> TransactionManagerDagSaveCommandReport {
    let mut queue_erased = Vec::new();
    for entry in &outcome.accepted {
        if entry.erased_from_queue {
            queue_erased.push(hash_command(entry.hash.0));
        }
    }
    TransactionManagerDagSaveCommandReport { queue_erased }
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

pub(crate) fn consensus_verify_transaction_fact_from_ffi_fact(
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
