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

#[cfg(test)]
use crate::ffi::rustaxa_ffi::TransactionQueueConfig;
#[cfg(test)]
use crate::ffi::rustaxa_ffi::TransactionQueueHash;
use crate::ffi::rustaxa_ffi::{
    GasPricerConfig, TransactionManagerAdmissionCommandReport, TransactionManagerAdmissionResult,
    TransactionManagerAdmissionShellIntent, TransactionManagerDagSaveCommandReport,
    TransactionManagerFinalChainAdmissionFact, TransactionManagerGasEstimationFact,
    TransactionManagerGasEstimationPlan, TransactionManagerHashCommand,
    TransactionManagerPublicAdmissionCommandReport, TransactionManagerPublicInsertResult,
    TransactionManagerTransactionView, TransactionManagerTransactionViewPlan,
    TransactionManagerTransactionViewRequest, TransactionManagerValidatedInsertRuntimeFact,
    TransactionManagerVerifyTransactionFact, TransactionManagerVerifyTransactionOutcome,
    TransactionQueueAccountNonceFact as BridgeTransactionQueueAccountNonceFact,
    TransactionQueueInsertInput, TransactionQueueStoredTransaction,
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
#[cfg(test)]
use rustaxa_consensus::transaction_queue::{
    TransactionQueueAccountNonceFact, TransactionQueuePurgeOutcome,
};
use rustaxa_consensus::transaction_queue::{TransactionQueueEntry, TransactionQueueInsertStatus};
#[cfg(test)]
use rustaxa_consensus::transaction_service::TransactionServiceConfig;
#[cfg(test)]
use rustaxa_consensus::transaction_service::TransactionServiceState;
use rustaxa_consensus::transaction_service::{
    DagTransactionSaveOutcome, TransactionServiceAccountNonceFact,
    TransactionServiceAdmissionReport, TransactionServiceFinalChainAdmissionFact,
    TransactionServiceGasEstimationPlan, TransactionServiceGasEstimationRequest,
    TransactionServicePublicAdmissionReport, TransactionServiceTransactionView,
    TransactionServiceTransactionViewPlan, TransactionServiceTransactionViewRequest,
    TransactionServiceValidatedAdmissionFact,
};
#[cfg(test)]
use rustaxa_storage::StatusField;
#[cfg(test)]
use rustaxa_storage::Storage;
#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::ops::{Deref, DerefMut};
#[cfg(test)]
use std::time::{Duration, Instant};

#[cfg(test)]
struct TransactionManagerRuntimeQueueCleanupPlan {
    non_proposable_expired: TransactionManagerRuntimeQueuePurgePlan,
    finalized_account_purged: TransactionManagerRuntimeQueuePurgePlan,
}

#[cfg(test)]
struct TransactionManagerRuntimeQueuePurgePlan {
    removed_hashes: Vec<TransactionQueueHash>,
}

#[cfg(test)]
type TransactionRuntimeState = TestTransactionServiceState;

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

#[cfg(test)]
pub(crate) struct TransactionManagerRuntimeQueueInsertOutcome {
    demoted_hashes: Vec<TransactionQueueHash>,
    overflow_removed_hashes: Vec<TransactionQueueHash>,
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
#[cfg(test)]
const TRANSACTION_QUEUE_DROP_WINDOW: Duration = Duration::from_secs(600);

fn hash_command(hash: [u8; 32]) -> TransactionManagerHashCommand {
    TransactionManagerHashCommand { hash }
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

#[cfg(test)]
fn runtime_hashes_to_bridge(hashes: Vec<H256>) -> Vec<TransactionQueueHash> {
    hashes
        .into_iter()
        .map(|hash| TransactionQueueHash { hash: hash.0 })
        .collect()
}

#[cfg(test)]
fn runtime_queue_purge_plan_from_consensus(
    outcome: TransactionQueuePurgeOutcome,
) -> TransactionManagerRuntimeQueuePurgePlan {
    TransactionManagerRuntimeQueuePurgePlan {
        removed_hashes: runtime_hashes_to_bridge(outcome.removed_hashes),
    }
}

#[cfg(test)]
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
                .unwrap_or_default();
            TransactionQueueAccountNonceFact {
                sender,
                account_found,
                account_nonce,
            }
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
    Box::new(TestTransactionServiceState {
        state: Some(Box::new(state)),
        cleanup_path: Some(path),
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

#[cfg(test)]
impl TestTransactionServiceState {
    /// Inserts transaction metadata and canonical bytes into the Rust-owned queue.
    #[cfg(test)]
    pub(crate) fn transaction_manager_runtime_queue_insert(
        &mut self,
        input: TransactionQueueInsertInput,
    ) -> Result<TransactionManagerRuntimeQueueInsertOutcome> {
        let proposable = input.proposable;
        let outcome = self
            .queue
            .insert(runtime_queue_entry_from_insert_input(&input), proposable)?;
        if matches!(outcome.status, TransactionQueueInsertStatus::Overflow)
            || !outcome.overflow_removed_hashes.is_empty()
        {
            self.last_drop_observed = Some(Instant::now());
        }
        Ok(TransactionManagerRuntimeQueueInsertOutcome {
            demoted_hashes: runtime_hashes_to_bridge(outcome.demoted_hashes),
            overflow_removed_hashes: runtime_hashes_to_bridge(outcome.overflow_removed_hashes),
        })
    }

    /// Returns true when the queue contains a transaction hash.
    #[cfg(test)]
    fn transaction_manager_runtime_queue_contains(&self, hash: &[u8; 32]) -> bool {
        self.queue.contains(H256::from(*hash))
    }

    #[cfg(test)]
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
    use crate::ffi::BridgeStorage;
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
        Ok(Box::new(TestTransactionServiceState {
            state: Some(Box::new(state)),
            cleanup_path: None,
        }))
    }

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
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
