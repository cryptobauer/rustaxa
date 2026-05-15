//! Deterministic transaction decision helpers for Rust-backed `TransactionManager` flows.
//!
//! The module currently owns transaction-manager decision boundaries used by C++ shims:
//! - proposer transaction packing (`packTrxs`)
//! - DAG-block transaction persistence planning (`saveTransactionsFromDagBlock`)
//! - finalized transaction status updates (`updateFinalizedTransactionsStatus`)
//! - finalized filtering (`excludeFinalizedTransactions` + `verifyTransactionsNotFinalized`)
//! - TransactionManager verification and pre-admission insert planning (`verifyTransaction`,
//!   `insertTransaction`)
//! - live transaction count, known membership decisions, and non-finalized/recently-finalized
//!   sidecar membership plus canonical RLP retention
//!
//! Planner functions remain side-effect free and deterministic. The
//! [`TransactionManagerSidecar`] state owns Rust-mode transaction count authority,
//! hash membership, and canonical transaction bytes; C++ still materializes transaction
//! objects, mutates the live queue/known-cache side effects, performs gas estimation,
//! and orchestrates lifecycle calls.

use crate::transaction_queue::TransactionQueueInsertStatus;
use anyhow::{Context, Result, ensure};
use ethereum_types::{H256, U256};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Candidate metadata supplied before C++ runs a gas estimate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionPackCandidate {
    /// Canonical transaction hash used by C++ to locate the live transaction.
    pub hash: H256,
    /// Declared transaction gas limit (`Transaction::getGas()`).
    pub declared_gas: u64,
}

/// Decision returned before C++ performs an expensive gas estimate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionPackCandidateDecision {
    /// True when C++ should estimate this candidate and feed the result back to Rust.
    pub should_estimate: bool,
}

/// Gas-estimation fact supplied after C++ runs the live estimate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionPackEstimate {
    /// Canonical transaction hash corresponding to the estimated candidate.
    pub hash: H256,
    /// Gas used returned by C++ FinalChain/EVM estimation.
    pub gas_used: u64,
}

/// Decision returned after Rust consumes a C++ gas estimate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionPackEstimateOutcome {
    /// Candidate hash echoed for C++ pointer/cache updates.
    pub hash: H256,
    /// True when C++ should include the live transaction in the proposal output.
    pub selected: bool,
    /// True when C++ should demote the transaction to non-proposable queue state.
    pub demote_to_non_proposable: bool,
    /// True when the legacy "remaining space cannot fit even the smallest transaction" rule stops the scan.
    pub stop: bool,
    /// Gas value to store beside the selected transaction in the C++ return value.
    pub gas_used: u64,
}

/// One candidate transaction fact from a DAG block, supplied by the C++ caller.
///
/// The caller supplies sender/account nonce and live-cache facts because those
/// sources are not Rust-owned yet. Rust owns the nonce-gated finalized-storage
/// lookup by invoking the callback passed to [`plan_transactions_from_dag_block`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DagTransactionSaveFact {
    /// Original input position in the C++ `SharedTransactions` slice.
    pub input_index: u64,
    /// Canonical transaction hash.
    pub hash: H256,
    /// Raw non-finalized transaction payload to persist when accepted.
    pub trx_rlp: Vec<u8>,
    /// The transaction `nonce` declared by `Transaction::getNonce()`.
    pub transaction_nonce: U256,
    /// The sender account nonce fact, typically from `FinalChain::getAccount`.
    pub sender_account_nonce: U256,
    /// True when the transaction is already tracked in the non-finalized DAG cache.
    pub in_non_finalized_cache: bool,
    /// True when the transaction is already tracked in the recently-finalized cache.
    pub in_recently_finalized_cache: bool,
}

/// Persistent payload for one accepted DAG-block transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DagTransactionSavePayload {
    /// Original input position of the accepted transaction.
    pub input_index: u64,
    /// Canonical transaction hash.
    pub hash: H256,
    /// Raw transaction RLP payload to persist in the non-finalized transaction column.
    pub trx_rlp: Vec<u8>,
}

/// Deterministic plan for one DAG block persistence sweep.
///
/// `accepted_transactions` is in first-accepted order and already de-duplicated by hash.
/// `target_transaction_count` is the manager-owned status counter value that should be
/// written as `StatusDbField::TrxCount` once persistence succeeds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DagTransactionSavePlan {
    pub accepted_transactions: Vec<DagTransactionSavePayload>,
    pub target_transaction_count: u64,
}

/// C++-originated finalized transaction fact supplied to Rust planning.
///
/// The caller supplies live cache membership because Rust does not yet own live
/// `TransactionManager` sidecars. The fact contains no transaction payload and
/// is stable across the CXX bridge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizedTransactionStatusFact {
    /// Original input position in the C++ `PeriodData` transaction list.
    pub input_index: u64,
    /// Canonical transaction hash.
    pub hash: H256,
    /// True when the transaction is currently still tracked in non-finalized DAG state.
    pub in_non_finalized_cache: bool,
}

/// Deterministic action for one finalized transaction after planning.
///
/// C++ uses `input_index` to resolve the live `SharedTransaction` pointer while
/// Rust controls which hashes participate in finalized-status side effects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizedTransactionStatusAction {
    /// Original input position to map back to C++ live structures.
    pub input_index: u64,
    /// Canonical transaction hash.
    pub hash: H256,
    /// True when this finalized transaction was removed from the live non-finalized sidecar.
    pub removed_non_finalized: bool,
}

/// Deterministic finalized-transaction status plan.
///
/// `accepted_transactions` is emitted in input order with one entry per input,
/// matching legacy `TransactionManager::updateFinalizedTransactionsStatus`.
/// `target_transaction_count` increments the current counter only for
/// transactions that were not present in the non-finalized DAG cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizedTransactionStatusPlan {
    pub accepted_transactions: Vec<FinalizedTransactionStatusAction>,
    pub target_transaction_count: u64,
    /// Some(stale_period) when `period > retention_window`, otherwise None.
    pub stale_period: Option<u64>,
    /// Legacy purge interval behavior: purge pending queue state every 100 periods.
    pub purge_transactions: bool,
}

/// Input for Rust-owned known-transaction admission decisions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionManagerKnownFact {
    /// Canonical transaction hash under test.
    pub hash: H256,
    /// True when the Rust-owned transaction queue known cache already contains the hash.
    pub queue_known: bool,
}

/// Input for finalized-transaction filtering decisions from legacy C++.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizedTransactionFilterFact {
    /// Original input position in the C++ hash slice.
    pub input_index: u64,
    /// Canonical transaction hash.
    pub hash: H256,
    /// True when the hash is already known in `recently_finalized_transactions_`.
    pub in_recently_finalized_cache: bool,
}

/// One filter action emitted for non-finalized hashes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizedTransactionFilterAction {
    /// Original input position.
    pub input_index: u64,
    /// Canonical transaction hash.
    pub hash: H256,
}

/// Plan for finalized filtering operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizedTransactionFilterPlan {
    /// Hashes that are not considered finalized by cache/storage checks.
    pub not_finalized: Vec<FinalizedTransactionFilterAction>,
}

/// Input for verify-not-finalized decisions from legacy C++.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifyNotFinalizedTransactionFact {
    /// Original input position in the C++ transaction list.
    pub input_index: u64,
    /// Canonical transaction hash.
    pub hash: H256,
    /// Transaction nonce from `Transaction::getNonce()`.
    pub transaction_nonce: U256,
    /// Sender account nonce from `FinalChain::getAccount().nonce`.
    pub sender_account_nonce: U256,
    /// True when the hash is already known in `recently_finalized_transactions_`.
    pub in_recently_finalized_cache: bool,
}

/// First finalized transaction observed while verifying a candidate transaction list.
///
/// The planner preserves the original input index so the C++ shim can validate
/// the returned hash against its live `SharedTransaction` vector before logging.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifyNotFinalizedTransactionFailure {
    /// Original input position of the first finalized transaction.
    pub input_index: u64,
    /// Canonical hash of the first finalized transaction.
    pub hash: H256,
}

/// Plan returned for `verifyTransactionsNotFinalized`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifyNotFinalizedTransactionPlan {
    /// `Some` when a transaction is finalized; `None` means all inputs passed.
    pub finalized: Option<VerifyNotFinalizedTransactionFailure>,
}

/// Builds a deterministic save plan from C++-supplied DAG transaction facts.
///
/// Filtering preserves legacy behavior from `TransactionManager::saveTransactionsFromDagBlock`:
/// - skip entries already known in non-finalized/recently-finalized in-memory sets
/// - skip duplicates within the same DAG block by hash
/// - when sender account nonce >= transaction nonce, consult storage through the provided callback
/// - accept all others
///
/// The returned `target_transaction_count` is computed by incrementing the supplied
/// `current_transaction_count` for each accepted transaction and errors on overflow.
pub fn plan_transactions_from_dag_block<F>(
    facts: Vec<DagTransactionSaveFact>,
    current_transaction_count: u64,
    mut is_finalized: F,
) -> Result<DagTransactionSavePlan>
where
    F: FnMut(H256) -> Result<bool>,
{
    let mut accepted_transactions = Vec::new();
    let mut accepted_hashes = HashSet::with_capacity(facts.len());
    let mut target_transaction_count = current_transaction_count;

    for fact in facts {
        ensure!(!fact.hash.is_zero(), "DAG transaction hash cannot be zero");

        if fact.in_non_finalized_cache
            || fact.in_recently_finalized_cache
            || !accepted_hashes.insert(fact.hash)
        {
            continue;
        }

        if fact.sender_account_nonce >= fact.transaction_nonce && is_finalized(fact.hash)? {
            continue;
        }

        target_transaction_count = target_transaction_count.checked_add(1).context(
            "transaction count overflow while planning DAG block transaction persistence",
        )?;

        accepted_transactions.push(DagTransactionSavePayload {
            input_index: fact.input_index,
            hash: fact.hash,
            trx_rlp: fact.trx_rlp,
        });
    }

    Ok(DagTransactionSavePlan {
        accepted_transactions,
        target_transaction_count,
    })
}

/// Builds a deterministic finalized-transaction status plan from C++ facts.
///
/// Inputs:
/// - `facts`: finalized transaction hashes in legacy period-data order plus
///   live non-finalized-cache membership.
/// - `current_transaction_count`: manager-owned `TrxCount` before the period.
/// - `period`: finalized PBFT period.
/// - `retention_window`: recently-finalized cache retention in PBFT periods.
///
/// Behavior:
/// - rejects zero hashes as malformed bridge input.
/// - preserves one action per input without de-duplicating.
/// - increments `target_transaction_count` only when a finalized transaction is
///   not found in the non-finalized DAG cache.
/// - reports stale cache eviction when `period > retention_window`.
/// - reports periodic queue purge when `period` is divisible by 100.
pub fn plan_finalized_transactions_status(
    facts: Vec<FinalizedTransactionStatusFact>,
    current_transaction_count: u64,
    period: u64,
    retention_window: u64,
) -> Result<FinalizedTransactionStatusPlan> {
    let mut target_transaction_count = current_transaction_count;
    let mut accepted_transactions = Vec::with_capacity(facts.len());

    for fact in facts {
        ensure!(
            !fact.hash.is_zero(),
            "finalized transaction hash cannot be zero"
        );

        if !fact.in_non_finalized_cache {
            target_transaction_count = target_transaction_count
                .checked_add(1)
                .context("transaction count overflow while planning finalized status updates")?;
        }

        accepted_transactions.push(FinalizedTransactionStatusAction {
            input_index: fact.input_index,
            hash: fact.hash,
            removed_non_finalized: fact.in_non_finalized_cache,
        });
    }

    Ok(FinalizedTransactionStatusPlan {
        accepted_transactions,
        target_transaction_count,
        stale_period: if period > retention_window {
            Some(period - retention_window)
        } else {
            None
        },
        purge_transactions: period.is_multiple_of(100),
    })
}

/// Builds deterministic filtering inputs for `TransactionManager::excludeFinalizedTransactions`.
///
/// Facts already mark cache hits for `recently_finalized_transactions_`, so Rust only checks
/// storage-backed finalized status for remaining candidates.
pub fn plan_exclude_finalized_transactions<F>(
    facts: Vec<FinalizedTransactionFilterFact>,
    mut is_finalized: F,
) -> Result<FinalizedTransactionFilterPlan>
where
    F: FnMut(H256) -> Result<bool>,
{
    let mut not_finalized = Vec::new();

    for fact in facts {
        ensure!(
            !fact.hash.is_zero(),
            "finalized filtering transaction hash cannot be zero"
        );

        if fact.in_recently_finalized_cache {
            continue;
        }

        if is_finalized(fact.hash).context("storage finalized lookup failed while filtering")? {
            continue;
        }

        not_finalized.push(FinalizedTransactionFilterAction {
            input_index: fact.input_index,
            hash: fact.hash,
        });
    }

    Ok(FinalizedTransactionFilterPlan { not_finalized })
}

/// Builds deterministic short-circuit output for
/// `TransactionManager::verifyTransactionsNotFinalized`.
///
/// For each transaction it mirrors the legacy logic:
/// - immediate failure when present in `recently_finalized_transactions_`
/// - storage finalized lookup only when `sender_account_nonce >= transaction_nonce`
pub fn plan_verify_not_finalized_transactions<F>(
    facts: Vec<VerifyNotFinalizedTransactionFact>,
    mut is_finalized: F,
) -> Result<VerifyNotFinalizedTransactionPlan>
where
    F: FnMut(H256) -> Result<bool>,
{
    for fact in facts {
        ensure!(
            !fact.hash.is_zero(),
            "finalized verification transaction hash cannot be zero"
        );

        if fact.in_recently_finalized_cache {
            return Ok(VerifyNotFinalizedTransactionPlan {
                finalized: Some(VerifyNotFinalizedTransactionFailure {
                    input_index: fact.input_index,
                    hash: fact.hash,
                }),
            });
        }

        if fact.sender_account_nonce >= fact.transaction_nonce
            && is_finalized(fact.hash).context("storage finalized lookup failed while verifying")?
        {
            return Ok(VerifyNotFinalizedTransactionPlan {
                finalized: Some(VerifyNotFinalizedTransactionFailure {
                    input_index: fact.input_index,
                    hash: fact.hash,
                }),
            });
        }
    }

    Ok(VerifyNotFinalizedTransactionPlan { finalized: None })
}

/// C++-originated facts required by TransactionManager::verifyTransaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionManagerVerifyTransactionFact {
    /// Transaction hash for input integrity checks.
    pub tx_hash: H256,
    /// Transaction chain id from the envelope header.
    pub chain_id: u64,
    /// Expected chain id configured by the current node.
    pub expected_chain_id: u64,
    /// Gas limit declared by the transaction.
    pub gas_limit: u64,
    /// Maximum allowed gas limit from genesis configuration.
    pub max_gas_limit: u64,
    /// Last finalized block number, used by C++ to resolve hardfork status.
    pub last_block_number: u64,
    /// Whether Cornus hardfork rules are active for this decision.
    pub cornus_active: bool,
    /// Whether intrinsic gas check was already computed in C++.
    pub intrinsic_gas_covered: bool,
    /// Whether signature validation already passed in C++.
    pub signature_valid: bool,
    /// Transaction gas price.
    pub gas_price: U256,
    /// Minimum gas price from configured genesis hardfork state.
    pub minimum_gas_price: U256,
}

/// Deterministic verify outcome for a single TransactionManager admission transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TransactionManagerVerifyTransactionStatus {
    /// Transaction passed all C++-mirrored admission checks.
    Accepted = 0,
    /// Transaction chain id does not match configured chain id.
    ChainIdMismatch = 1,
    /// Transaction gas exceeds configured Tx max gas limit.
    InvalidGas = 2,
    /// Cornus hardfork gate is active and intrinsic gas was not covered.
    IntrinsicGasNotCovered = 3,
    /// Signature validation failed upstream.
    InvalidSignature = 4,
    /// Gas price is below the minimum required by consensus config.
    GasPriceTooLow = 5,
}

impl TransactionManagerVerifyTransactionStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Accepted)
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// TransactionManager::verifyTransaction plan result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionManagerVerifyTransactionOutcome {
    /// Status selected from `TransactionManagerVerifyTransactionStatus`.
    pub status: TransactionManagerVerifyTransactionStatus,
}

/// C++-originated facts required by TransactionManager::insertTransaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionManagerInsertTransactionFact {
    /// Transaction hash for input integrity checks.
    pub tx_hash: H256,
    /// Whether this hash is already known in the live transaction pool.
    pub hash_known: bool,
    /// Post-queue insertion status returned from C++ queue insertion.
    pub queue_status: TransactionQueueInsertStatus,
    /// Whether a finalized transaction location was resolved by C++.
    pub has_finalized_period: bool,
    /// Finalized location period, used only when `has_finalized_period`.
    pub finalized_period: u64,
}

/// C++-originated facts required before mutating the live transaction queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionManagerValidatedInsertFact {
    /// Transaction hash for input integrity checks.
    pub tx_hash: H256,
    /// Transaction nonce.
    pub transaction_nonce: U256,
    /// Transaction cost: value + gas_price * gas_limit.
    pub transaction_cost: U256,
    /// Gas limit declared by the transaction.
    pub gas_limit: u64,
    /// Configured DAG proposal gas limit.
    pub propose_dag_gas_limit: u64,
    /// Whether C++ is allowed to keep non-proposable transactions.
    pub insert_non_proposable: bool,
    /// Whether the hash is already tracked in non-finalized DAG sidecars.
    pub in_non_finalized_cache: bool,
    /// Whether the hash is already tracked in recently-finalized sidecars.
    pub in_recently_finalized_cache: bool,
    /// Whether C++ found a sender account in FinalChain state.
    pub account_found: bool,
    /// Sender account nonce when `account_found` is true.
    pub account_nonce: U256,
    /// Sender account balance when `account_found` is true.
    pub account_balance: U256,
}

/// Rust decision for `TransactionManager::insertValidatedTransaction`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionManagerValidatedInsertPlan {
    /// Status to return directly when `should_insert_queue` is false.
    pub status: TransactionQueueInsertStatus,
    /// True when C++ should call the live transaction queue insertion API.
    pub should_insert_queue: bool,
    /// Proposable flag to pass to the live transaction queue when inserting.
    pub queue_proposable: bool,
    /// True when C++ should emit `transaction_added_` before queue insertion.
    pub emit_transaction_added: bool,
}

/// One ordered sidecar payload lookup result for C++ transaction materialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionManagerSidecarLookup {
    /// Original caller-provided request position.
    pub input_index: u64,
    /// Canonical transaction hash used as the sidecar key.
    pub hash: H256,
    /// True when `trx_rlp` was found in Rust-owned sidecar state.
    pub found: bool,
    /// Sidecar source: 0 missing, 1 non-finalized, 2 recently-finalized.
    pub source: u8,
    /// Canonical transaction RLP payload, when found.
    pub trx_rlp: Vec<u8>,
}

/// Recovery insertion entry consumed by the Rust-owned TransactionManager sidecar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionManagerSidecarRecoveryEntry {
    /// Canonical transaction hash used as the sidecar key.
    pub hash: H256,
    /// True when recovery source marked this entry stale/finalized.
    pub finalized: bool,
    /// Canonical transaction RLP payload.
    pub trx_rlp: Vec<u8>,
}

/// Rust-owned live transaction sidecar state for TransactionManager.
///
/// Execution boundary:
/// - owns the authoritative Rust-mode transaction count loaded from/persisted to storage
/// - owns canonical RLP payload retention for live non-finalized and recently-finalized hashes
/// - supports hash membership and known-admission checks used by C++ plumbing
/// - provides ordered payload lookups without exposing internal map ordering
/// - applies finalized transitions and stale eviction deterministically
/// - accepts non-finalized recovery payloads while skipping stale finalized rows
#[derive(Clone, Debug, Default)]
pub struct TransactionManagerSidecar {
    transaction_count: u64,
    non_finalized: HashMap<H256, Vec<u8>>,
    recently_finalized: HashMap<H256, Vec<u8>>,
    recently_finalized_periods: BTreeMap<u64, Vec<H256>>,
}

impl TransactionManagerSidecarLookup {
    /// Missing sidecar lookup source.
    pub const SOURCE_MISSING: u8 = 0;
    /// Non-finalized sidecar lookup source.
    pub const SOURCE_NON_FINALIZED: u8 = 1;
    /// Recently-finalized sidecar lookup source.
    pub const SOURCE_RECENTLY_FINALIZED: u8 = 2;
}

impl TransactionManagerSidecar {
    /// Creates a TransactionManager live sidecar seeded with the persisted transaction count.
    pub fn new(initial_transaction_count: u64) -> Self {
        Self {
            transaction_count: initial_transaction_count,
            ..Self::default()
        }
    }

    /// Returns the authoritative Rust-mode transaction count.
    pub fn transaction_count(&self) -> u64 {
        self.transaction_count
    }

    /// Replaces the authoritative Rust-mode transaction count after a committed storage write.
    pub fn set_transaction_count(&mut self, transaction_count: u64) {
        self.transaction_count = transaction_count;
    }

    /// Returns true when a transaction should be treated as known by Rust-mode admission.
    ///
    /// The queue still owns wall-clock known-cache expiry and supplies `queue_known`;
    /// Rust folds that fact together with manager sidecar membership so DAG/recently
    /// finalized payloads participate in one deterministic admission decision.
    pub fn is_transaction_known(&self, fact: TransactionManagerKnownFact) -> Result<bool> {
        ensure!(
            !fact.hash.is_zero(),
            "known transaction hash cannot be zero"
        );
        Ok(fact.queue_known
            || self.contains_non_finalized(fact.hash)
            || self.contains_recently_finalized(fact.hash))
    }

    /// Inserts or updates one non-finalized transaction payload in canonical form.
    pub fn insert_non_finalized(&mut self, hash: H256, trx_rlp: Vec<u8>) -> Result<()> {
        ensure!(!hash.is_zero(), "sidecar non-finalized hash cannot be zero");
        self.recently_finalized.remove(&hash);
        self.non_finalized.insert(hash, trx_rlp);
        Ok(())
    }

    /// Inserts or updates one recently-finalized transaction payload in canonical form.
    pub fn insert_recently_finalized(
        &mut self,
        period: u64,
        hash: H256,
        trx_rlp: Vec<u8>,
    ) -> Result<bool> {
        ensure!(
            !hash.is_zero(),
            "sidecar recently-finalized hash cannot be zero"
        );
        let removed_non_finalized = self.non_finalized.remove(&hash).is_some();
        self.recently_finalized.insert(hash, trx_rlp);
        self.recently_finalized_periods
            .entry(period)
            .or_default()
            .push(hash);
        Ok(removed_non_finalized)
    }

    /// Removes one non-finalized sidecar payload.
    pub fn remove_non_finalized(&mut self, hash: H256) -> bool {
        self.non_finalized.remove(&hash).is_some()
    }

    /// True when the hash exists in Rust-owned non-finalized sidecar state.
    pub fn contains_non_finalized(&self, hash: H256) -> bool {
        self.non_finalized.contains_key(&hash)
    }

    /// True when the hash exists in Rust-owned recently-finalized sidecar state.
    pub fn contains_recently_finalized(&self, hash: H256) -> bool {
        self.recently_finalized.contains_key(&hash)
    }

    /// Returns the number of Rust-owned non-finalized transaction sidecars.
    pub fn non_finalized_size(&self) -> usize {
        self.non_finalized.len()
    }

    /// Returns payloads for ordered hash requests, preserving input order.
    pub fn lookup_payloads_ordered(
        &self,
        requests: Vec<(u64, H256)>,
    ) -> Result<Vec<TransactionManagerSidecarLookup>> {
        let mut out = Vec::with_capacity(requests.len());
        for (input_index, hash) in requests {
            ensure!(!hash.is_zero(), "sidecar lookup hash cannot be zero");
            let (source, payload) = if let Some(payload) = self.non_finalized.get(&hash) {
                (
                    TransactionManagerSidecarLookup::SOURCE_NON_FINALIZED,
                    Some(payload),
                )
            } else if let Some(payload) = self.recently_finalized.get(&hash) {
                (
                    TransactionManagerSidecarLookup::SOURCE_RECENTLY_FINALIZED,
                    Some(payload),
                )
            } else {
                (TransactionManagerSidecarLookup::SOURCE_MISSING, None)
            };
            out.push(TransactionManagerSidecarLookup {
                input_index,
                hash,
                found: payload.is_some(),
                source,
                trx_rlp: payload.cloned().unwrap_or_default(),
            });
        }
        Ok(out)
    }

    /// Moves finalized hashes from non-finalized to recently-finalized sidecar state.
    pub fn apply_finalized_transition(&mut self, period: u64, hashes: Vec<H256>) -> Result<()> {
        for hash in hashes {
            ensure!(
                !hash.is_zero(),
                "sidecar finalized transition hash cannot be zero"
            );
            if let Some(trx_rlp) = self.non_finalized.get(&hash).cloned() {
                self.insert_recently_finalized(period, hash, trx_rlp)?;
            }
        }
        Ok(())
    }

    /// Evicts recently-finalized payloads from one stale period.
    ///
    /// Returns the number of payloads removed from the recently-finalized hash map.
    pub fn evict_recently_finalized_stale_period(&mut self, stale_period: u64) -> usize {
        self.recently_finalized_periods
            .remove(&stale_period)
            .map(|hashes| {
                hashes
                    .into_iter()
                    .filter(|hash| self.recently_finalized.remove(hash).is_some())
                    .count()
            })
            .unwrap_or_default()
    }

    /// Inserts recovery payloads into non-finalized sidecar state.
    ///
    /// Entries marked finalized are skipped to avoid restoring stale payloads.
    /// Returns the number of inserted non-finalized payloads.
    pub fn insert_recovery_entries(
        &mut self,
        entries: Vec<TransactionManagerSidecarRecoveryEntry>,
    ) -> Result<usize> {
        let mut inserted = 0usize;
        for entry in entries {
            ensure!(
                !entry.hash.is_zero(),
                "sidecar recovery hash cannot be zero"
            );
            if entry.finalized {
                self.non_finalized.remove(&entry.hash);
                continue;
            }
            self.recently_finalized.remove(&entry.hash);
            self.non_finalized.insert(entry.hash, entry.trx_rlp);
            inserted += 1;
        }
        Ok(inserted)
    }
}

/// Deterministic outcome of a transaction insert admission plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TransactionManagerInsertTransactionStatus {
    /// Transaction inserted (proposable or non-proposable).
    Accepted = 0,
    /// Transaction already known in the live transaction pool.
    AlreadyKnown = 1,
    /// Transaction already finalized at a known period.
    AlreadyFinalized = 2,
    /// Queue mutation or post-insert state prevented acceptance.
    CouldNotInsert = 3,
}

impl TransactionManagerInsertTransactionStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Accepted)
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// TransactionManager::insertTransaction plan result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionManagerInsertTransactionOutcome {
    /// Outcome status.
    pub status: TransactionManagerInsertTransactionStatus,
    /// Finalized period when `status == AlreadyFinalized`.
    pub finalized_period: Option<u64>,
}

/// Builds a deterministic plan for TransactionManager::verifyTransaction.
pub fn plan_verify_transaction(
    fact: TransactionManagerVerifyTransactionFact,
) -> Result<TransactionManagerVerifyTransactionOutcome> {
    ensure!(
        !fact.tx_hash.is_zero(),
        "verify transaction hash cannot be zero"
    );

    if fact.chain_id != fact.expected_chain_id {
        return Ok(TransactionManagerVerifyTransactionOutcome {
            status: TransactionManagerVerifyTransactionStatus::ChainIdMismatch,
        });
    }

    if fact.max_gas_limit < fact.gas_limit {
        return Ok(TransactionManagerVerifyTransactionOutcome {
            status: TransactionManagerVerifyTransactionStatus::InvalidGas,
        });
    }

    if fact.cornus_active && !fact.intrinsic_gas_covered {
        return Ok(TransactionManagerVerifyTransactionOutcome {
            status: TransactionManagerVerifyTransactionStatus::IntrinsicGasNotCovered,
        });
    }

    if !fact.signature_valid {
        return Ok(TransactionManagerVerifyTransactionOutcome {
            status: TransactionManagerVerifyTransactionStatus::InvalidSignature,
        });
    }

    if fact.gas_price < fact.minimum_gas_price {
        return Ok(TransactionManagerVerifyTransactionOutcome {
            status: TransactionManagerVerifyTransactionStatus::GasPriceTooLow,
        });
    }

    Ok(TransactionManagerVerifyTransactionOutcome {
        status: TransactionManagerVerifyTransactionStatus::Accepted,
    })
}

/// Builds a deterministic plan for TransactionManager::insertTransaction.
///
/// Behavior mirrors the upstream shim shape:
/// - immediate rejection for known hashes
/// - queue status mapping for successful/unsuccessful inserts
/// - finalized-period reason when queue reports `Known` and finalization lookup exists
pub fn plan_insert_transaction(
    fact: TransactionManagerInsertTransactionFact,
) -> Result<TransactionManagerInsertTransactionOutcome> {
    ensure!(
        !fact.tx_hash.is_zero(),
        "insert transaction hash cannot be zero"
    );

    if fact.hash_known {
        return Ok(TransactionManagerInsertTransactionOutcome {
            status: TransactionManagerInsertTransactionStatus::AlreadyKnown,
            finalized_period: None,
        });
    }

    match fact.queue_status {
        TransactionQueueInsertStatus::Inserted => Ok(TransactionManagerInsertTransactionOutcome {
            status: TransactionManagerInsertTransactionStatus::Accepted,
            finalized_period: None,
        }),
        TransactionQueueInsertStatus::InsertedNonProposable => {
            Ok(TransactionManagerInsertTransactionOutcome {
                status: TransactionManagerInsertTransactionStatus::CouldNotInsert,
                finalized_period: None,
            })
        }
        TransactionQueueInsertStatus::Overflow => Ok(TransactionManagerInsertTransactionOutcome {
            status: TransactionManagerInsertTransactionStatus::CouldNotInsert,
            finalized_period: None,
        }),
        TransactionQueueInsertStatus::Known => {
            if fact.has_finalized_period {
                Ok(TransactionManagerInsertTransactionOutcome {
                    status: TransactionManagerInsertTransactionStatus::AlreadyFinalized,
                    finalized_period: Some(fact.finalized_period),
                })
            } else {
                Ok(TransactionManagerInsertTransactionOutcome {
                    status: TransactionManagerInsertTransactionStatus::CouldNotInsert,
                    finalized_period: None,
                })
            }
        }
    }
}

/// Builds a deterministic pre-mutation plan for `insertValidatedTransaction`.
///
/// C++ supplies live cache and account facts while Rust decides whether the
/// transaction is immediately `Known`, should be inserted as proposable, or
/// should be retained only as non-proposable when the caller allows that.
pub fn plan_validated_insert(
    fact: TransactionManagerValidatedInsertFact,
) -> Result<TransactionManagerValidatedInsertPlan> {
    ensure!(
        !fact.tx_hash.is_zero(),
        "validated insert transaction hash cannot be zero"
    );

    if fact.in_non_finalized_cache || fact.in_recently_finalized_cache {
        return Ok(TransactionManagerValidatedInsertPlan {
            status: TransactionQueueInsertStatus::Known,
            should_insert_queue: false,
            queue_proposable: false,
            emit_transaction_added: false,
        });
    }

    let mut proposable = true;
    if fact.account_found {
        if fact.account_nonce > fact.transaction_nonce {
            if !fact.insert_non_proposable {
                return Ok(TransactionManagerValidatedInsertPlan {
                    status: TransactionQueueInsertStatus::Known,
                    should_insert_queue: false,
                    queue_proposable: false,
                    emit_transaction_added: false,
                });
            }
            proposable = false;
        }

        if fact.account_balance < fact.transaction_cost {
            if !fact.insert_non_proposable {
                return Ok(TransactionManagerValidatedInsertPlan {
                    status: TransactionQueueInsertStatus::Known,
                    should_insert_queue: false,
                    queue_proposable: false,
                    emit_transaction_added: false,
                });
            }
            proposable = false;
        }
    } else {
        if !fact.insert_non_proposable {
            return Ok(TransactionManagerValidatedInsertPlan {
                status: TransactionQueueInsertStatus::Known,
                should_insert_queue: false,
                queue_proposable: false,
                emit_transaction_added: false,
            });
        }
        proposable = false;
    }

    if fact.propose_dag_gas_limit < fact.gas_limit {
        if !fact.insert_non_proposable {
            return Ok(TransactionManagerValidatedInsertPlan {
                status: TransactionQueueInsertStatus::Known,
                should_insert_queue: false,
                queue_proposable: false,
                emit_transaction_added: false,
            });
        }
        proposable = false;
    }

    Ok(TransactionManagerValidatedInsertPlan {
        status: if proposable {
            TransactionQueueInsertStatus::Inserted
        } else {
            TransactionQueueInsertStatus::InsertedNonProposable
        },
        should_insert_queue: true,
        queue_proposable: proposable,
        emit_transaction_added: proposable,
    })
}

/// Stateful planner for one `TransactionManager::packTrxs` invocation.
///
/// Invariants:
/// - `min_transaction_gas` must be non-zero.
/// - `total_weight` changes only after a valid gas estimate is accepted.
/// - fit and stop arithmetic uses wrapping operations to match legacy unsigned C++ behavior.
/// - the planner never stores live transaction data or mutates queue state.
#[derive(Clone, Debug)]
pub struct TransactionPackingPlanner {
    weight_limit: u64,
    min_transaction_gas: u64,
    total_weight: u64,
}

impl TransactionPackingPlanner {
    /// Creates a planner for one proposal-packing pass.
    pub fn new(weight_limit: u64, min_transaction_gas: u64) -> Result<Self> {
        ensure!(
            min_transaction_gas != 0,
            "minimum transaction gas must be non-zero"
        );
        Ok(Self {
            weight_limit,
            min_transaction_gas,
            total_weight: 0,
        })
    }

    /// Returns the maximum number of ordered queue candidates C++ should fetch for this packing pass.
    pub fn max_candidate_count(&self) -> u64 {
        self.weight_limit / self.min_transaction_gas
    }

    /// Decides whether a candidate can proceed to live gas estimation.
    pub fn consider_candidate(
        &self,
        candidate: TransactionPackCandidate,
    ) -> Result<TransactionPackCandidateDecision> {
        ensure!(
            !candidate.hash.is_zero(),
            "transaction candidate hash cannot be zero"
        );
        Ok(TransactionPackCandidateDecision {
            should_estimate: self.total_weight.wrapping_add(candidate.declared_gas)
                <= self.weight_limit,
        })
    }

    /// Consumes a C++ gas-estimation result and returns the required live-state action.
    pub fn record_estimate(
        &mut self,
        estimate: TransactionPackEstimate,
    ) -> Result<TransactionPackEstimateOutcome> {
        ensure!(
            !estimate.hash.is_zero(),
            "transaction estimate hash cannot be zero"
        );
        if estimate.gas_used < self.min_transaction_gas {
            return Ok(TransactionPackEstimateOutcome {
                hash: estimate.hash,
                selected: false,
                demote_to_non_proposable: true,
                stop: false,
                gas_used: estimate.gas_used,
            });
        }

        self.total_weight = self.total_weight.wrapping_add(estimate.gas_used);
        Ok(TransactionPackEstimateOutcome {
            hash: estimate.hash,
            selected: true,
            demote_to_non_proposable: false,
            stop: self.weight_limit.wrapping_sub(self.total_weight) <= self.min_transaction_gas,
            gas_used: estimate.gas_used,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(hash: u8, gas: u64) -> TransactionPackCandidate {
        TransactionPackCandidate {
            hash: H256::from([hash; 32]),
            declared_gas: gas,
        }
    }

    #[test]
    fn max_candidate_count_matches_weight_limit_floor() {
        let planner = TransactionPackingPlanner::new(63_000, 21_000).unwrap();
        assert_eq!(planner.max_candidate_count(), 3);
    }

    #[test]
    fn candidate_fit_uses_accepted_estimate_weight() {
        let mut planner = TransactionPackingPlanner::new(63_000, 21_000).unwrap();

        assert!(
            planner
                .consider_candidate(tx(1, 40_000))
                .unwrap()
                .should_estimate
        );
        let first = planner
            .record_estimate(TransactionPackEstimate {
                hash: H256::from([1; 32]),
                gas_used: 40_000,
            })
            .unwrap();
        assert!(first.selected);
        assert!(!first.stop);

        assert!(
            !planner
                .consider_candidate(tx(2, 24_000))
                .unwrap()
                .should_estimate
        );
        assert!(
            planner
                .consider_candidate(tx(3, 23_000))
                .unwrap()
                .should_estimate
        );
    }

    #[test]
    fn invalid_estimate_requests_non_proposable_demote_without_weight_change() {
        let mut planner = TransactionPackingPlanner::new(63_000, 21_000).unwrap();
        let invalid = planner
            .record_estimate(TransactionPackEstimate {
                hash: H256::from([1; 32]),
                gas_used: 20_999,
            })
            .unwrap();

        assert!(!invalid.selected);
        assert!(invalid.demote_to_non_proposable);
        assert!(!invalid.stop);
        assert!(
            planner
                .consider_candidate(tx(2, 63_000))
                .unwrap()
                .should_estimate
        );
    }

    #[test]
    fn stop_matches_legacy_remaining_minimum_rule() {
        let mut planner = TransactionPackingPlanner::new(63_000, 21_000).unwrap();
        let outcome = planner
            .record_estimate(TransactionPackEstimate {
                hash: H256::from([1; 32]),
                gas_used: 42_000,
            })
            .unwrap();

        assert!(outcome.selected);
        assert!(outcome.stop);
    }

    fn save_fact(
        hash: u8,
        trx_nonce: u64,
        sender_nonce: u64,
        in_non_finalized_cache: bool,
        in_recently_finalized_cache: bool,
    ) -> DagTransactionSaveFact {
        DagTransactionSaveFact {
            input_index: hash as u64,
            hash: H256::from([hash; 32]),
            trx_rlp: vec![hash],
            transaction_nonce: U256::from(trx_nonce),
            sender_account_nonce: U256::from(sender_nonce),
            in_non_finalized_cache,
            in_recently_finalized_cache,
        }
    }

    fn finalized_status_fact(
        input_index: u64,
        hash: u8,
        in_non_finalized_cache: bool,
    ) -> FinalizedTransactionStatusFact {
        FinalizedTransactionStatusFact {
            input_index,
            hash: H256::from([hash; 32]),
            in_non_finalized_cache,
        }
    }

    #[test]
    fn dag_block_save_plan_filters_known_flags_duplicates_and_nonce_gates_finalization() {
        let plan = plan_transactions_from_dag_block(
            vec![
                save_fact(1, 5, 4, false, false),
                save_fact(1, 5, 4, false, false),
                save_fact(2, 9, 11, true, false),
                save_fact(3, 9, 11, false, true),
                save_fact(4, 1, 5, false, false),
                save_fact(5, 5, 11, false, false),
                save_fact(6, 2, 1, false, false),
            ],
            12,
            |hash| Ok(hash == H256::from([4; 32])),
        )
        .unwrap();

        assert_eq!(plan.target_transaction_count, 15);
        assert_eq!(
            plan.accepted_transactions
                .iter()
                .map(|payload| (payload.input_index, payload.hash))
                .collect::<Vec<_>>(),
            vec![
                (1, H256::from([1; 32])),
                (5, H256::from([5; 32])),
                (6, H256::from([6; 32])),
            ]
        );
        assert_eq!(plan.accepted_transactions[0].trx_rlp, vec![1]);
        assert_eq!(plan.accepted_transactions[1].trx_rlp, vec![5]);
        assert_eq!(plan.accepted_transactions[2].trx_rlp, vec![6]);
    }

    #[test]
    fn dag_block_save_plan_overflow_is_reported_before_persistence() {
        let result = plan_transactions_from_dag_block(
            vec![save_fact(1, 1, 0, false, false)],
            u64::MAX,
            |_| Ok(false),
        );

        assert!(result.is_err());
    }

    #[test]
    fn dag_block_save_plan_only_checks_storage_when_nonce_requires_it() {
        let mut looked_up = Vec::new();
        let plan = plan_transactions_from_dag_block(
            vec![
                save_fact(1, 5, 4, false, false),
                save_fact(2, 5, 5, false, false),
                save_fact(3, 5, 8, false, false),
            ],
            0,
            |hash| {
                looked_up.push(hash);
                Ok(hash == H256::from([2; 32]))
            },
        )
        .unwrap();

        assert_eq!(looked_up, vec![H256::from([2; 32]), H256::from([3; 32])]);
        assert_eq!(
            plan.accepted_transactions
                .iter()
                .map(|payload| (payload.input_index, payload.hash))
                .collect::<Vec<_>>(),
            vec![(1, H256::from([1; 32])), (3, H256::from([3; 32]))]
        );
    }

    #[test]
    fn finalized_status_plan_counts_only_when_not_in_non_finalized_cache() {
        let plan = plan_finalized_transactions_status(
            vec![
                finalized_status_fact(0, 1, false),
                finalized_status_fact(1, 2, true),
                finalized_status_fact(2, 3, false),
            ],
            7,
            220,
            20,
        )
        .unwrap();

        assert_eq!(plan.target_transaction_count, 9);
        assert_eq!(
            plan.accepted_transactions
                .iter()
                .map(|action| (action.input_index, action.hash))
                .collect::<Vec<_>>(),
            vec![
                (0, H256::from([1; 32])),
                (1, H256::from([2; 32])),
                (2, H256::from([3; 32]))
            ]
        );
    }

    #[test]
    fn finalized_status_plan_includes_stale_period_and_purge_flag() {
        let plan = plan_finalized_transactions_status(
            vec![
                finalized_status_fact(0, 1, false),
                finalized_status_fact(1, 2, false),
            ],
            0,
            200,
            10,
        )
        .unwrap();

        assert_eq!(plan.stale_period, Some(190));
        assert!(plan.purge_transactions);
    }

    #[test]
    fn finalized_status_plan_omits_stale_period_when_window_not_exceeded() {
        let plan =
            plan_finalized_transactions_status(vec![finalized_status_fact(0, 1, false)], 0, 5, 10)
                .unwrap();

        assert_eq!(plan.stale_period, None);
        assert!(!plan.purge_transactions);
    }

    #[test]
    fn finalized_status_plan_overflow_is_reported_before_persistence() {
        let result = plan_finalized_transactions_status(
            vec![finalized_status_fact(0, 1, false)],
            u64::MAX,
            200,
            10,
        );

        assert!(result.is_err());
    }

    fn finalized_filter_fact(
        input_index: u64,
        hash: u8,
        in_recently_finalized_cache: bool,
    ) -> FinalizedTransactionFilterFact {
        FinalizedTransactionFilterFact {
            input_index,
            hash: H256::from([hash; 32]),
            in_recently_finalized_cache,
        }
    }

    fn verify_not_finalized_fact(
        input_index: u64,
        hash: u8,
        transaction_nonce: u64,
        sender_account_nonce: u64,
        in_recently_finalized_cache: bool,
    ) -> VerifyNotFinalizedTransactionFact {
        VerifyNotFinalizedTransactionFact {
            input_index,
            hash: H256::from([hash; 32]),
            transaction_nonce: U256::from(transaction_nonce),
            sender_account_nonce: U256::from(sender_account_nonce),
            in_recently_finalized_cache,
        }
    }

    #[test]
    fn finalized_filter_plan_excludes_recent_cache_and_storage() {
        let mut lookup_count = 0;
        let plan = plan_exclude_finalized_transactions(
            vec![
                finalized_filter_fact(0, 1, false),
                finalized_filter_fact(1, 2, true),
                finalized_filter_fact(2, 3, false),
                finalized_filter_fact(3, 4, false),
            ],
            |hash| {
                lookup_count += 1;
                Ok(hash == H256::from([3; 32]))
            },
        )
        .unwrap();

        assert_eq!(lookup_count, 3);
        assert_eq!(
            plan.not_finalized,
            vec![
                FinalizedTransactionFilterAction {
                    input_index: 0,
                    hash: H256::from([1; 32]),
                },
                FinalizedTransactionFilterAction {
                    input_index: 3,
                    hash: H256::from([4; 32]),
                },
            ]
        );
    }

    #[test]
    fn finalize_filter_plan_rejects_zero_hash_inputs() {
        let result =
            plan_exclude_finalized_transactions(vec![finalized_filter_fact(0, 0, false)], |_| {
                Ok(false)
            });
        assert!(result.is_err());
    }

    #[test]
    fn verify_not_finalized_plan_short_circuits_on_cache_before_storage() {
        let mut lookup_count = 0;
        let plan = plan_verify_not_finalized_transactions(
            vec![
                verify_not_finalized_fact(0, 1, 2, 8, true),
                verify_not_finalized_fact(1, 2, 1, 2, false),
            ],
            |_hash| {
                lookup_count += 1;
                Ok(true)
            },
        )
        .unwrap();

        assert_eq!(lookup_count, 0);
        assert_eq!(
            plan.finalized,
            Some(VerifyNotFinalizedTransactionFailure {
                input_index: 0,
                hash: H256::from([1; 32]),
            })
        );
    }

    #[test]
    fn verify_not_finalized_plan_skips_storage_lookup_when_sender_nonce_is_below_transaction_nonce()
    {
        let mut lookup_count = 0;
        let plan = plan_verify_not_finalized_transactions(
            vec![
                verify_not_finalized_fact(0, 1, 10, 1, false),
                verify_not_finalized_fact(1, 2, 2, 4, false),
            ],
            |_hash| {
                lookup_count += 1;
                Ok(true)
            },
        )
        .unwrap();

        assert_eq!(lookup_count, 1);
        assert_eq!(
            plan.finalized,
            Some(VerifyNotFinalizedTransactionFailure {
                input_index: 1,
                hash: H256::from([2; 32]),
            })
        );
    }

    fn verify_transaction_fact(
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
            tx_hash: H256::from([tx_hash; 32]),
            chain_id,
            expected_chain_id,
            gas_limit,
            max_gas_limit,
            cornus_active,
            intrinsic_gas_covered,
            signature_valid,
            gas_price: U256::from(gas_price),
            minimum_gas_price: U256::from(minimum_gas_price),
            last_block_number,
        }
    }

    #[test]
    fn verify_transaction_plan_applies_chain_id_and_gas_gate() {
        assert_eq!(
            plan_verify_transaction(verify_transaction_fact(
                1, 1, 1, 21_000, 100_000, false, true, true, 1, 1, 0,
            ))
            .unwrap()
            .status,
            TransactionManagerVerifyTransactionStatus::Accepted
        );

        assert_eq!(
            plan_verify_transaction(verify_transaction_fact(
                1, 2, 1, 21_000, 100_000, false, true, true, 1, 1, 0,
            ))
            .unwrap()
            .status,
            TransactionManagerVerifyTransactionStatus::ChainIdMismatch
        );
    }

    #[test]
    fn verify_transaction_plan_enforces_intrinsic_and_signature_gates() {
        assert_eq!(
            plan_verify_transaction(verify_transaction_fact(
                1, 1, 1, 21_000, 100_000, true, false, true, 1, 1, 0,
            ))
            .unwrap()
            .status,
            TransactionManagerVerifyTransactionStatus::IntrinsicGasNotCovered
        );

        assert_eq!(
            plan_verify_transaction(verify_transaction_fact(
                1, 1, 1, 21_000, 100_000, false, true, false, 1, 1, 0,
            ))
            .unwrap()
            .status,
            TransactionManagerVerifyTransactionStatus::InvalidSignature
        );
    }

    #[test]
    fn verify_transaction_plan_enforces_minimum_gas_price() {
        assert_eq!(
            plan_verify_transaction(verify_transaction_fact(
                1, 1, 1, 21_000, 100_000, false, true, true, 4, 8, 0,
            ))
            .unwrap()
            .status,
            TransactionManagerVerifyTransactionStatus::GasPriceTooLow
        );
    }

    fn validated_insert_fact(
        tx_hash: u8,
        transaction_nonce: u64,
        transaction_cost: u64,
        gas_limit: u64,
        propose_dag_gas_limit: u64,
        insert_non_proposable: bool,
        in_non_finalized_cache: bool,
        in_recently_finalized_cache: bool,
        account_found: bool,
        account_nonce: u64,
        account_balance: u64,
    ) -> TransactionManagerValidatedInsertFact {
        TransactionManagerValidatedInsertFact {
            tx_hash: H256::from([tx_hash; 32]),
            transaction_nonce: U256::from(transaction_nonce),
            transaction_cost: U256::from(transaction_cost),
            gas_limit,
            propose_dag_gas_limit,
            insert_non_proposable,
            in_non_finalized_cache,
            in_recently_finalized_cache,
            account_found,
            account_nonce: U256::from(account_nonce),
            account_balance: U256::from(account_balance),
        }
    }

    fn insert_transaction_fact(
        tx_hash: u8,
        hash_known: bool,
        queue_status: TransactionQueueInsertStatus,
        has_finalized_period: bool,
        finalized_period: u64,
    ) -> TransactionManagerInsertTransactionFact {
        TransactionManagerInsertTransactionFact {
            tx_hash: H256::from([tx_hash; 32]),
            hash_known,
            queue_status,
            has_finalized_period,
            finalized_period,
        }
    }

    #[test]
    fn validated_insert_plan_rejects_live_cache_hits_before_queue_mutation() {
        let plan = plan_validated_insert(validated_insert_fact(
            1, 1, 10, 21_000, 100_000, true, true, false, true, 0, 100,
        ))
        .unwrap();

        assert_eq!(plan.status, TransactionQueueInsertStatus::Known);
        assert!(!plan.should_insert_queue);
    }

    #[test]
    fn validated_insert_plan_marks_nonce_balance_and_gas_failures_non_proposable_when_allowed() {
        let nonce_plan = plan_validated_insert(validated_insert_fact(
            1, 1, 10, 21_000, 100_000, true, false, false, true, 2, 100,
        ))
        .unwrap();
        assert!(nonce_plan.should_insert_queue);
        assert!(!nonce_plan.queue_proposable);
        assert!(!nonce_plan.emit_transaction_added);

        let balance_plan = plan_validated_insert(validated_insert_fact(
            2, 1, 200, 21_000, 100_000, true, false, false, true, 0, 100,
        ))
        .unwrap();
        assert!(balance_plan.should_insert_queue);
        assert!(!balance_plan.queue_proposable);

        let gas_plan = plan_validated_insert(validated_insert_fact(
            3, 1, 10, 200_000, 100_000, true, false, false, true, 0, 100,
        ))
        .unwrap();
        assert!(gas_plan.should_insert_queue);
        assert!(!gas_plan.queue_proposable);
    }

    #[test]
    fn validated_insert_plan_returns_known_for_non_proposable_facts_when_not_allowed() {
        let plan = plan_validated_insert(validated_insert_fact(
            1, 1, 10, 21_000, 100_000, false, false, false, false, 0, 0,
        ))
        .unwrap();

        assert_eq!(plan.status, TransactionQueueInsertStatus::Known);
        assert!(!plan.should_insert_queue);
    }

    #[test]
    fn validated_insert_plan_accepts_proposable_transactions() {
        let plan = plan_validated_insert(validated_insert_fact(
            1, 1, 10, 21_000, 100_000, false, false, false, true, 0, 100,
        ))
        .unwrap();

        assert_eq!(plan.status, TransactionQueueInsertStatus::Inserted);
        assert!(plan.should_insert_queue);
        assert!(plan.queue_proposable);
        assert!(plan.emit_transaction_added);
    }

    #[test]
    fn insert_transaction_plan_prefers_known_hash_fast_path() {
        assert_eq!(
            plan_insert_transaction(insert_transaction_fact(
                1,
                true,
                TransactionQueueInsertStatus::Inserted,
                false,
                0,
            ))
            .unwrap()
            .status,
            TransactionManagerInsertTransactionStatus::AlreadyKnown
        );
    }

    #[test]
    fn insert_transaction_plan_maps_queue_known_to_finalized_or_reject() {
        assert_eq!(
            plan_insert_transaction(insert_transaction_fact(
                1,
                false,
                TransactionQueueInsertStatus::Known,
                true,
                100,
            ))
            .unwrap(),
            TransactionManagerInsertTransactionOutcome {
                status: TransactionManagerInsertTransactionStatus::AlreadyFinalized,
                finalized_period: Some(100),
            }
        );

        assert_eq!(
            plan_insert_transaction(insert_transaction_fact(
                1,
                false,
                TransactionQueueInsertStatus::Known,
                false,
                0,
            ))
            .unwrap()
            .status,
            TransactionManagerInsertTransactionStatus::CouldNotInsert
        );
    }

    #[test]
    fn insert_transaction_plan_accepts_queue_insert_and_non_proposable_status() {
        assert_eq!(
            plan_insert_transaction(insert_transaction_fact(
                1,
                false,
                TransactionQueueInsertStatus::Inserted,
                false,
                0,
            ))
            .unwrap()
            .status,
            TransactionManagerInsertTransactionStatus::Accepted
        );
        assert_eq!(
            plan_insert_transaction(insert_transaction_fact(
                2,
                false,
                TransactionQueueInsertStatus::InsertedNonProposable,
                false,
                0,
            ))
            .unwrap()
            .status,
            TransactionManagerInsertTransactionStatus::CouldNotInsert
        );
        assert_eq!(
            plan_insert_transaction(insert_transaction_fact(
                3,
                false,
                TransactionQueueInsertStatus::Overflow,
                false,
                0,
            ))
            .unwrap()
            .status,
            TransactionManagerInsertTransactionStatus::CouldNotInsert
        );
    }

    #[test]
    fn sidecar_lookup_preserves_order_and_uses_both_live_sets() {
        let mut sidecar = TransactionManagerSidecar::new(0);
        sidecar
            .insert_non_finalized(H256::from([1; 32]), vec![0x11])
            .unwrap();
        sidecar
            .insert_non_finalized(H256::from([2; 32]), vec![0x22])
            .unwrap();
        sidecar
            .apply_finalized_transition(9, vec![H256::from([2; 32])])
            .unwrap();

        let out = sidecar
            .lookup_payloads_ordered(vec![
                (7, H256::from([2; 32])),
                (8, H256::from([3; 32])),
                (9, H256::from([1; 32])),
            ])
            .unwrap();
        assert_eq!(
            out.into_iter()
                .map(|entry| {
                    (
                        entry.input_index,
                        entry.hash,
                        entry.found,
                        entry.source,
                        entry.trx_rlp,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (
                    7,
                    H256::from([2; 32]),
                    true,
                    TransactionManagerSidecarLookup::SOURCE_RECENTLY_FINALIZED,
                    vec![0x22],
                ),
                (
                    8,
                    H256::from([3; 32]),
                    false,
                    TransactionManagerSidecarLookup::SOURCE_MISSING,
                    Vec::<u8>::new(),
                ),
                (
                    9,
                    H256::from([1; 32]),
                    true,
                    TransactionManagerSidecarLookup::SOURCE_NON_FINALIZED,
                    vec![0x11],
                ),
            ]
        );
    }

    #[test]
    fn sidecar_finalized_transition_and_stale_eviction_are_bounded_by_period() {
        let mut sidecar = TransactionManagerSidecar::new(0);
        sidecar
            .insert_non_finalized(H256::from([1; 32]), vec![0x11])
            .unwrap();
        sidecar
            .insert_non_finalized(H256::from([2; 32]), vec![0x22])
            .unwrap();

        sidecar
            .apply_finalized_transition(10, vec![H256::from([1; 32]), H256::from([2; 32])])
            .unwrap();
        assert!(sidecar.contains_recently_finalized(H256::from([1; 32])));
        assert!(!sidecar.contains_non_finalized(H256::from([1; 32])));

        assert_eq!(sidecar.evict_recently_finalized_stale_period(9), 0);
        assert_eq!(sidecar.evict_recently_finalized_stale_period(10), 2);
        assert!(!sidecar.contains_recently_finalized(H256::from([1; 32])));
    }

    #[test]
    fn sidecar_recovery_insertion_skips_finalized_entries() {
        let mut sidecar = TransactionManagerSidecar::new(0);
        let inserted = sidecar
            .insert_recovery_entries(vec![
                TransactionManagerSidecarRecoveryEntry {
                    hash: H256::from([1; 32]),
                    finalized: false,
                    trx_rlp: vec![0x11],
                },
                TransactionManagerSidecarRecoveryEntry {
                    hash: H256::from([2; 32]),
                    finalized: true,
                    trx_rlp: vec![0x22],
                },
            ])
            .unwrap();

        assert_eq!(inserted, 1);
        assert!(sidecar.contains_non_finalized(H256::from([1; 32])));
        assert!(!sidecar.contains_non_finalized(H256::from([2; 32])));
    }

    #[test]
    fn sidecar_owns_count_and_known_decision() {
        let mut sidecar = TransactionManagerSidecar::new(41);
        assert_eq!(sidecar.transaction_count(), 41);
        sidecar.set_transaction_count(42);
        assert_eq!(sidecar.transaction_count(), 42);

        sidecar
            .insert_non_finalized(H256::from([1; 32]), vec![0x11])
            .unwrap();
        sidecar
            .insert_recently_finalized(7, H256::from([2; 32]), vec![0x22])
            .unwrap();

        assert!(
            sidecar
                .is_transaction_known(TransactionManagerKnownFact {
                    hash: H256::from([1; 32]),
                    queue_known: false,
                })
                .unwrap()
        );
        assert!(
            sidecar
                .is_transaction_known(TransactionManagerKnownFact {
                    hash: H256::from([2; 32]),
                    queue_known: false,
                })
                .unwrap()
        );
        assert!(
            sidecar
                .is_transaction_known(TransactionManagerKnownFact {
                    hash: H256::from([3; 32]),
                    queue_known: true,
                })
                .unwrap()
        );
        assert!(
            !sidecar
                .is_transaction_known(TransactionManagerKnownFact {
                    hash: H256::from([4; 32]),
                    queue_known: false,
                })
                .unwrap()
        );
    }
}
