use crate::gas_pricer::{GasPriceOracle, GasPricerConfig};
use crate::transaction_manager::TransactionManagerSidecar;
use crate::transaction_manager::{DagTransactionSaveFact, plan_transactions_from_dag_block};
use crate::transaction_packing_service::{
    TransactionPackingCandidate, TransactionPackingEffect, TransactionPackingEstimate,
    TransactionPackingEstimateRequest, TransactionPackingOwner, TransactionPackingRequest,
    TransactionPackingSelection, TransactionPackingService,
};
use crate::transaction_queue::TransactionQueue;
use crate::transaction_queue::TransactionQueueDemoteStatus;
use crate::transaction_storage::{
    NonFinalizedTransactionStoragePayload, STORED_TRANSACTION_SOURCE_FINALIZED_REGULAR,
    STORED_TRANSACTION_SOURCE_FINALIZED_SYSTEM, STORED_TRANSACTION_SOURCE_MISSING,
    STORED_TRANSACTION_SOURCE_PENDING, StoredTransactionLookupRequest,
    append_non_finalized_transactions_to_batch, load_stored_transactions, transaction_finalized,
};
use anyhow::{Context, Result, anyhow, ensure};
use ethereum_types::{H160, H256, U256};
use rustaxa_storage::StorageWriteBatch;
use rustaxa_storage::{StatusField, Storage};
use rustaxa_types::LegacyTransactionEnvelope;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

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
/// count and gas-history restoration succeed. Its public fields are a temporary
/// CRW-12 bridge escape hatch for short-lived Rust adapters; callers must hold a
/// [`TransactionServiceGuard`] and must not retain references across external
/// executor callbacks.
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
pub(crate) struct DagTransactionSaveInput {
    pub input_index: u64,
    pub hash: H256,
    pub transaction_rlp: Vec<u8>,
    pub transaction_nonce: U256,
    pub sender_account_nonce: U256,
}

/// Native accepted-transaction publication result.
#[derive(Clone, Copy)]
pub struct DagTransactionSaveAccepted {
    /// Canonical transaction identity.
    pub hash: H256,
    /// Whether publication erased the transaction from the live queue.
    pub erased_from_queue: bool,
}

/// Native live-state result for a committed DAG transaction save.
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
    /// Restores the complete state used by [`TransactionService`].
    ///
    /// This is public only for the temporary bridge adapter and its focused
    /// mechanics tests. Production publication must use
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
        let candidate_limit = TransactionPackingService::candidate_limit(
            params.weight_limit,
            min_transaction_gas,
            params.total_transaction_shards,
            params.node_transaction_shard,
            params.shard_period_interval,
        )?;
        let candidates = self
            .queue
            .ordered_transactions(candidate_limit)
            .into_iter()
            .map(|entry| {
                let cached_gas_used = self
                    .sidecar
                    .gas_estimation_cache_get(entry.hash, params.proposal_period)
                    .context("TM_RUNTIME_PACK_GAS_ESTIMATION_CACHE_GET")?
                    .map(|cached| cached.gas_used);
                Ok((entry, cached_gas_used))
            })
            .collect::<Result<Vec<_>>>()?;
        let step = self
            .transaction_packing
            .prepare(TransactionPackingRequest {
                owner,
                weight_limit: params.weight_limit,
                min_transaction_gas,
                proposal_period: params.proposal_period,
                estimate_gas_limit,
                last_block_number,
                total_shards: params.total_transaction_shards,
                node_shard: params.node_transaction_shard,
                shard_period_interval: params.shard_period_interval,
                candidates: candidates
                    .into_iter()
                    .map(|(entry, cached_gas_used)| TransactionPackingCandidate {
                        entry,
                        cached_gas_used,
                    })
                    .collect(),
            })
            .context("TM_RUNTIME_PACK_PREPARE")?;
        let request_estimates = step
            .request_estimates
            .iter()
            .cloned()
            .map(transaction_estimate_request)
            .collect::<Result<Vec<_>>>()?;
        let (selected_transactions, demoted_hashes) =
            apply_packing_step_effects(self, &step, "TM_RUNTIME_PACK_PREPARE")?;
        if !demoted_hashes.is_empty() {
            self.transaction_packing
                .acknowledge_demotions(owner, demoted_hashes.clone())?;
        }
        Ok(TransactionServiceProposerPackPrepared {
            request_estimates,
            selected_transactions,
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
        let step = self
            .transaction_packing
            .finalize(owner, estimates)
            .context("TM_RUNTIME_PACK_FINALIZE")?;
        let (selected_transactions, _demoted_hashes) =
            apply_packing_step_effects(self, &step, "TM_RUNTIME_PACK_FINALIZE")?;
        Ok(TransactionServiceProposerPackFinalized {
            selected_transactions,
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
        hash: u8,
    ) -> TransactionServiceTransactionViewRequest {
        TransactionServiceTransactionViewRequest {
            input_index,
            hash: [hash; 32],
        }
    }

    fn signed_legacy_transaction_rlp(signing_key: &SigningKey) -> Vec<u8> {
        let chain_id = 2999_u64;
        let mut unsigned = RlpStream::new_list(9);
        unsigned.append(&U256::from(1));
        unsigned.append(&U256::from(2));
        unsigned.append(&21_000_u64);
        unsigned.append(&H160::repeat_byte(0x44));
        unsigned.append(&U256::from(3));
        unsigned.append(&Vec::<u8>::new());
        unsigned.append(&U256::from(chain_id));
        unsigned.append(&U256::zero());
        unsigned.append(&U256::zero());
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(keccak256(&unsigned.out()).as_bytes())
            .expect("test transaction signing must succeed");
        let signature = signature.to_bytes();
        let mut signed = RlpStream::new_list(9);
        signed.append(&U256::from(1));
        signed.append(&U256::from(2));
        signed.append(&21_000_u64);
        signed.append(&H160::repeat_byte(0x44));
        signed.append(&U256::from(3));
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
        let mut runtime = service.lock()?;

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
        let mut runtime = service.lock()?;

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
        let runtime = service.lock()?;

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
                queue_max_size: 8,
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
        }

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

        let runtime = service.lock()?;
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
            .apply_finalized_transition(7, vec![ethereum_types::H256::from_low_u64_be(3)])
            .expect("sidecar transition should seed recently-finalized source");
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

        let mut period_txs = RlpStream::new_list(1);
        period_txs.append_raw(&[0x55], 1);
        let mut period_data = RlpStream::new_list(5);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&period_txs.out(), 1);
        period_data.append_raw(&[0xC0], 1);
        runtime
            .storage
            .period()
            .write(9, &period_data.out())
            .expect("period source should persist finalized tx");

        let plan = runtime.lookup_transaction_views(
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

        let mut runtime = service.lock()?;
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

        let mut period_txs = RlpStream::new_list(1);
        period_txs.append_raw(&transaction_rlp, 1);
        let mut period_data = RlpStream::new_list(5);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&period_txs.out(), 1);
        period_data.append_raw(&[0xC0], 1);
        runtime
            .storage
            .period()
            .write(1, &period_data.out())
            .expect("proposal period data should persist");

        let plan = runtime.lookup_proposal_transaction_views_with_account_nonce_facts(
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
        assert!(plan.views[0].found);
        assert!(plan.views[0].old_finalized);
        assert_eq!(
            plan.views[0].source,
            TM_TRANSACTION_VIEW_SOURCE_STORAGE_FINALIZED_REGULAR
        );
        assert!(plan.views[0].tx_rlp.is_empty());

        let permissive_plan = runtime.lookup_proposal_transaction_views_with_account_nonce_facts(
            1,
            vec![TransactionServiceTransactionViewRequest {
                input_index: 10,
                hash: transaction_hash.to_fixed_bytes(),
            }],
            vec![],
            0,
        )?;
        assert!(permissive_plan.views[0].found);

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
        let mut runtime = service.lock()?;
        let corrupt_hash = H256::from_low_u64_be(8);

        runtime
            .storage
            .transaction()
            .write_location(corrupt_hash, 2, 0, false)
            .expect("proposal storage location should persist");
        let mut period_txs = RlpStream::new_list(1);
        period_txs.append_raw(&[0xFF], 1);
        let mut period_data = RlpStream::new_list(5);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&period_txs.out(), 1);
        period_data.append_raw(&[0xC0], 1);
        runtime
            .storage
            .period()
            .write(2, &period_data.out())
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
