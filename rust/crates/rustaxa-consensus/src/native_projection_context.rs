//! Invocation-time ordinary account evidence for opt-in concrete execution.
//!
//! The canonical projection retains its legacy encoding. This Rust-only sidecar
//! supplies the frame facts and signed balances that encoding cannot represent.
//! It is bound to the exact application request and projection bytes, whose
//! lineage must already have passed ordinary concrete pair validation. These
//! are executor facts, not intermediate trie proofs or publication authority.

use crate::concrete_state_projection::{
    FinalChainConcreteStateProjection, concrete_state_bytes_digest,
    encode_concrete_state_projection,
};
use crate::native_session::{
    FinalChainNativeAccount, FinalChainNativeCallKind, FinalChainNativeOrdinaryMutation,
    FinalChainNativeRawMutation, FinalChainNativeRawOperation, FinalChainNativeRequest,
};
use anyhow::{Result, ensure};
use num_bigint::BigUint;
use std::collections::{BTreeMap, BTreeSet};

/// Current ordinary facts and ordered effects for exactly one native call.
///
/// A validator must independently execute the selected native kernel against
/// these account facts and compare both effect streams. It then discards that
/// scratch ordinary state: transaction account projections own which ordinary
/// effects survived enclosing rollback. Semantic/native state persists across
/// invocation replay independently of ordinary rollback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainNativeInvocationContext {
    /// Complete prepared frame request, including inherited static mode.
    pub request: FinalChainNativeRequest,
    /// Unique invocation-time accounts actually supplied to the native kernel.
    /// Absence must have zero nonce and balance; no missing read means zero.
    pub accounts: Vec<([u8; 20], FinalChainNativeAccount)>,
    /// Unique exact raw values read from the current native lane. Read-only
    /// owner/metadata keys belong here even when no mutation targets them.
    pub raw_reads: Vec<FinalChainNativeRawRead>,
    /// Original ordinary effects before any enclosing rollback.
    pub ordinary_mutations: Vec<FinalChainNativeOrdinaryMutation>,
    /// Original raw effects in native operation order.
    pub raw_mutations: Vec<FinalChainNativeRawMutation>,
}

/// One exact raw observation from the current execution journal.
/// Reads must be unique per phase; repeated reads observe the same pre-outcome
/// journal state. Raw effects from earlier calls remain visible across rollback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainNativeRawRead {
    /// Native contract owning the logical row.
    pub address: [u8; 20],
    /// Logical, unhashed storage key.
    pub key: rustaxa_types::concrete_state::ConcreteStorageKey,
    /// Exact classified raw bytes, including present empty tombstones.
    pub value: rustaxa_types::concrete_state::ConcreteRead<Vec<u8>>,
}

/// One actual terminal phase in executor order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalChainNativeRewardsPhase {
    Distribution {
        source_period: u64,
        ordinal: usize,
        executed: bool,
    },
    DeferredEndBlock,
    SlashingCleanup,
}

/// Half-open ranges into the original ordered raw and ordinary effect vectors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainNativeRewardsBoundary {
    pub phase: FinalChainNativeRewardsPhase,
    pub raw_start: usize,
    pub raw_end: usize,
    pub ordinary_start: usize,
    pub ordinary_end: usize,
}

/// Current-state evidence and ordered effects for the single terminal rewards phase.
/// The application supplies the opaque reward plan separately; no reward facts
/// or publication authority are invented by this executor context.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FinalChainNativeRewardsContext {
    /// Actual terminal phase boundaries, in execution order.
    pub boundaries: Vec<FinalChainNativeRewardsBoundary>,
    /// Exact current ordinary accounts read after all transactions.
    pub accounts: Vec<([u8; 20], FinalChainNativeAccount)>,
    /// Exact current native raw reads before rewards/end-block effects apply.
    pub raw_reads: Vec<FinalChainNativeRawRead>,
    /// Ordered custody credits and other ordinary reward effects.
    pub ordinary_mutations: Vec<FinalChainNativeOrdinaryMutation>,
    /// Ordered rewards and end-block native raw effects.
    pub raw_mutations: Vec<FinalChainNativeRawMutation>,
}

/// One executor lifecycle event. Frame and creation identifiers are event indexes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalChainLifecycleEvent {
    FrameOpened {
        parent: Option<usize>,
    },
    AccountCreated {
        address: [u8; 20],
        frame: Option<usize>,
        saved_absent: bool,
    },
    NativeStarted {
        transaction: u32,
        sequence: u64,
    },
    NativeObserved {
        transaction: u32,
        sequence: u64,
    },
    TransactionEnd {
        observations: usize,
    },
    AccountReset {
        address: [u8; 20],
        creation: usize,
        rollback_frame: usize,
    },
}

/// Claimed events for one transaction and executor instance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainLifecycleReceipt {
    pub transaction_position: u32,
    pub transaction_hash: [u8; 32],
    pub token: u64,
    pub event_count: usize,
    pub events: Vec<FinalChainLifecycleEvent>,
}

/// Trusted adapter input retained independently of mutable context claims.
/// This is not a verified token for use by an untrusted caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainLifecycleAuthority {
    pub request_id: [u8; 32],
    pub state_api_epoch: u64,
    pub projection_hash: [u8; 32],
    pub receipts: Vec<FinalChainLifecycleReceipt>,
}

/// Exact request/projection binding for the optional Rust execution context.
///
/// An adapter using staged native execution must supply a context even when
/// its invocation list is empty. Legacy leaves may omit this opt-in sidecar.
/// It is neither persisted separately nor used during recovery: the approved
/// publication intent and existing concrete provenance remain authoritative.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainNativeProjectionContext {
    /// Request that constructed the staged native session.
    pub request_id: [u8; 32],
    /// Digest of the exact canonical thirteen-field projection bytes.
    pub projection_hash: [u8; 32],
    /// Exactly one entry per consensus invocation in global sequence order.
    pub invocations: Vec<FinalChainNativeInvocationContext>,
    /// The single rewards/end-block phase of the same native session.
    pub rewards: FinalChainNativeRewardsContext,
    /// Lifecycle claims; checked against separately retained executor evidence.
    pub lifecycle: Vec<FinalChainLifecycleReceipt>,
}

/// Compares claims with separately retained trusted executor evidence.
pub(crate) fn validate_lifecycle_authority(
    context: &FinalChainNativeProjectionContext,
    authority: &FinalChainLifecycleAuthority,
    request_id: [u8; 32],
    epoch: u64,
    projection_hash: [u8; 32],
    transactions: &[(u32, [u8; 32])],
) -> Result<()> {
    ensure!(
        authority.request_id == request_id
            && authority.state_api_epoch == epoch
            && authority.projection_hash == projection_hash,
        "FINAL_CHAIN_NATIVE_AUTHORITY_BINDING_MISMATCH"
    );
    ensure!(
        authority.receipts.len() == transactions.len() && context.lifecycle == authority.receipts,
        "FINAL_CHAIN_NATIVE_AUTHORITY_RECEIPTS_MISMATCH"
    );
    for (receipt, (position, hash)) in authority.receipts.iter().zip(transactions) {
        ensure!(
            receipt.transaction_position == *position
                && receipt.transaction_hash == *hash
                && receipt.token != 0
                && receipt.event_count == receipt.events.len()
                && matches!(
                    receipt.events.last(),
                    Some(FinalChainLifecycleEvent::TransactionEnd { .. })
                ),
            "FINAL_CHAIN_NATIVE_AUTHORITY_TRANSACTION_MISMATCH"
        );
    }
    Ok(())
}

impl FinalChainNativeProjectionContext {
    /// Validates binding and every frame fact represented by the legacy codec.
    ///
    /// The caller first validates projection lineage, then calls this method
    /// before semantic replay or persisting a commit intent. Only the bounded
    /// DPoS CALL/STATICCALL route is admitted. This does not validate kernel
    /// results or authenticate invocation-time ordinary account facts.
    pub(crate) fn validate_binding(
        &self,
        request_id: [u8; 32],
        projection: &FinalChainConcreteStateProjection,
    ) -> Result<()> {
        ensure!(
            self.request_id == request_id,
            "FINAL_CHAIN_NATIVE_CONTEXT_REQUEST_MISMATCH"
        );
        ensure!(
            self.projection_hash
                == concrete_state_bytes_digest(&encode_concrete_state_projection(projection)),
            "FINAL_CHAIN_NATIVE_CONTEXT_PROJECTION_MISMATCH"
        );
        ensure!(
            self.invocations.len() == projection.invocations.len(),
            "FINAL_CHAIN_NATIVE_CONTEXT_COUNT_MISMATCH"
        );
        for (context, invocation) in self.invocations.iter().zip(&projection.invocations) {
            let request = &context.request;
            let call_type = match request.kind {
                FinalChainNativeCallKind::Call => 0,
                FinalChainNativeCallKind::StaticCall => 3,
                _ => anyhow::bail!("FINAL_CHAIN_NATIVE_CONTEXT_CALL_KIND_UNSUPPORTED"),
            };
            ensure!(
                request.id.transaction.as_u32() as u64 == invocation.transaction_index
                    && request.id.sequence == invocation.sequence
                    && request.period.as_u64() == projection.post_transaction_state.period
                    && request.depth == invocation.depth
                    && call_type == invocation.call_type
                    && request.caller == invocation.caller
                    && request.contract == invocation.contract
                    && request.value.value() == &BigUint::from_bytes_be(&invocation.value)
                    && request.input == invocation.input
                    && request.supplied_gas.as_u64() == invocation.supplied_gas,
                "FINAL_CHAIN_NATIVE_CONTEXT_FRAME_MISMATCH"
            );
            ensure!(
                matches!(
                    request.contract,
                    crate::final_chain::DPOS_CONTRACT_ADDRESS
                        | crate::final_chain::SLASHING_CONTRACT_ADDRESS
                ) && request.state_address == request.contract,
                "FINAL_CHAIN_NATIVE_CONTEXT_ADDRESS_UNSUPPORTED"
            );
            ensure!(
                request.kind != FinalChainNativeCallKind::StaticCall
                    || (request.is_static && request.value.is_zero()),
                "FINAL_CHAIN_NATIVE_CONTEXT_STATIC_MISMATCH"
            );
            ensure!(
                invocation.disposition != crate::concrete_state_projection::FINAL_CHAIN_CONCRETE_INVOCATION_PARENT_FRAME_REVERTED
                    || invocation.error.is_empty(),
                "FINAL_CHAIN_NATIVE_CONTEXT_DISPOSITION_MISMATCH"
            );
            validate_read_sets(&context.accounts, &context.raw_reads)?;
        }
        validate_read_sets(&self.rewards.accounts, &self.rewards.raw_reads)?;
        let lineage = validate_raw_lineage(self);
        #[cfg(feature = "synthetic-fixtures")]
        dispatch_lineage_diagnostic(
            &lineage,
            self,
            request_id,
            projection,
            std::env::var_os("RUSTAXA_NATIVE_LINEAGE_DIAGNOSTIC").as_deref()
                == Some(std::ffi::OsStr::new("invalid_signature_a")),
            &mut std::io::stderr().lock(),
        );
        lineage?;
        Ok(())
    }
}

/// Checks structural current-read evidence before any staged replay begins.
pub(crate) fn validate_read_sets(
    accounts: &[([u8; 20], FinalChainNativeAccount)],
    raw_reads: &[FinalChainNativeRawRead],
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for (address, account) in accounts {
        ensure!(
            seen.insert(*address),
            "FINAL_CHAIN_NATIVE_CONTEXT_DUPLICATE_ACCOUNT"
        );
        ensure!(
            account.exists
                || (account.nonce == rustaxa_types::FinalChainNonce::zero()
                    && account.balance == num_bigint::BigInt::default()),
            "FINAL_CHAIN_NATIVE_CONTEXT_ABSENT_ACCOUNT_NONZERO"
        );
    }
    let mut seen = BTreeSet::new();
    for read in raw_reads {
        ensure!(
            matches!(
                read.address,
                crate::final_chain::DPOS_CONTRACT_ADDRESS
                    | crate::final_chain::SLASHING_CONTRACT_ADDRESS
            ),
            "FINAL_CHAIN_NATIVE_CONTEXT_RAW_ADDRESS_UNSUPPORTED"
        );
        ensure!(
            seen.insert((read.address, read.key)),
            "FINAL_CHAIN_NATIVE_CONTEXT_DUPLICATE_RAW_READ"
        );
    }
    Ok(())
}

type RawIdentity = ([u8; 20], rustaxa_types::concrete_state::ConcreteStorageKey);

/// The same ordered result feeds lineage and both storage checks.
#[derive(Default)]
pub(crate) struct LifecycleRawFold {
    pub transactions: BTreeMap<u32, LifecycleTransactionRaw>,
    pub final_values: BTreeMap<RawIdentity, Vec<u8>>,
    pub absent_accounts: BTreeSet<[u8; 20]>,
}

#[derive(Default)]
pub(crate) struct LifecycleTransactionRaw {
    pub values: BTreeMap<RawIdentity, Vec<u8>>,
    pub initially_absent: BTreeSet<RawIdentity>,
    pub created: BTreeSet<RawIdentity>,
    pub erased: BTreeMap<RawIdentity, Vec<LifecycleErasedMutation>>,
}

#[derive(Clone, Debug)]
pub(crate) struct LifecycleErasedMutation {
    pub sequence: u64,
    pub observed: usize,
    pub reset: usize,
    pub value: Vec<u8>,
}

/// Folds only authority-checked events. This does not change native replay state.
pub(crate) fn fold_lifecycle_raw(
    context: &FinalChainNativeProjectionContext,
) -> Result<LifecycleRawFold> {
    use rustaxa_types::concrete_state::ConcreteRead;
    let mut result = LifecycleRawFold::default();
    let mut known: BTreeMap<RawIdentity, (Option<u32>, ConcreteRead<Vec<u8>>)> = BTreeMap::new();
    let mut next = 0usize;
    let mut previous_tx = None;
    for receipt in &context.lifecycle {
        let tx = receipt.transaction_position;
        ensure!(
            previous_tx.is_none_or(|v| v < tx),
            "FINAL_CHAIN_NATIVE_LIFECYCLE_TRANSACTION_ORDER"
        );
        previous_tx = Some(tx);
        ensure!(
            receipt.event_count == receipt.events.len(),
            "FINAL_CHAIN_NATIVE_LIFECYCLE_EVENT_COUNT"
        );
        let mut frames = BTreeMap::<usize, Option<usize>>::new();
        let mut creations = BTreeMap::<usize, ([u8; 20], Option<usize>)>::new();
        let mut consumed = BTreeSet::new();
        let mut active: Option<usize> = None;
        let mut observed = 0usize;
        let mut ended = false;
        let mut delta = LifecycleTransactionRaw::default();
        let mut pending = BTreeMap::<RawIdentity, BTreeMap<u64, (usize, Vec<u8>)>>::new();
        for (ordinal, event) in receipt.events.iter().enumerate() {
            ensure!(!ended, "FINAL_CHAIN_NATIVE_LIFECYCLE_AFTER_END");
            match event {
                FinalChainLifecycleEvent::FrameOpened { parent } => {
                    ensure!(
                        active.is_none() && parent.is_none_or(|id| frames.contains_key(&id)),
                        "FINAL_CHAIN_NATIVE_LIFECYCLE_FRAME"
                    );
                    frames.insert(ordinal, *parent);
                }
                FinalChainLifecycleEvent::AccountCreated {
                    address,
                    frame,
                    saved_absent,
                } => {
                    ensure!(
                        *saved_absent && frame.is_none_or(|id| frames.contains_key(&id)),
                        "FINAL_CHAIN_NATIVE_LIFECYCLE_CREATION"
                    );
                    creations.insert(ordinal, (*address, *frame));
                }
                FinalChainLifecycleEvent::AccountReset {
                    address,
                    creation,
                    rollback_frame,
                } => {
                    ensure!(
                        active.is_none() && consumed.insert(*creation),
                        "FINAL_CHAIN_NATIVE_LIFECYCLE_RESET"
                    );
                    let (owner, mut frame) = creations.get(creation).copied().ok_or_else(|| {
                        anyhow::anyhow!("FINAL_CHAIN_NATIVE_LIFECYCLE_CREATION_MISSING")
                    })?;
                    ensure!(
                        owner == *address && frames.contains_key(rollback_frame),
                        "FINAL_CHAIN_NATIVE_LIFECYCLE_RESET_OWNER"
                    );
                    while let Some(id) = frame {
                        if id == *rollback_frame {
                            break;
                        }
                        frame = frames.get(&id).copied().flatten();
                    }
                    ensure!(
                        frame == Some(*rollback_frame),
                        "FINAL_CHAIN_NATIVE_LIFECYCLE_RESET_ANCESTRY"
                    );
                    known.retain(|(owner, _), _| owner != address);
                    result.final_values.retain(|(owner, _), _| owner != address);
                    result.absent_accounts.insert(*address);
                    let erased_keys = pending
                        .keys()
                        .filter(|(owner, _)| owner == address)
                        .cloned()
                        .collect::<Vec<_>>();
                    for key in erased_keys {
                        let observations = pending.get(&key).expect("collected key exists");
                        delta.erased.insert(
                            key,
                            observations
                                .iter()
                                .map(|(sequence, (observed, value))| LifecycleErasedMutation {
                                    sequence: *sequence,
                                    observed: *observed,
                                    reset: ordinal,
                                    value: value.clone(),
                                })
                                .collect(),
                        );
                    }
                    delta.values.retain(|(owner, _), _| owner != address);
                    delta.initially_absent.retain(|(owner, _)| owner != address);
                    delta.created.retain(|(owner, _)| owner != address);
                }
                FinalChainLifecycleEvent::NativeStarted {
                    transaction,
                    sequence,
                } => {
                    ensure!(
                        active.is_none() && *transaction == tx,
                        "FINAL_CHAIN_NATIVE_LIFECYCLE_START"
                    );
                    let call = context.invocations.get(next).ok_or_else(|| {
                        anyhow::anyhow!("FINAL_CHAIN_NATIVE_LIFECYCLE_CALL_MISSING")
                    })?;
                    ensure!(
                        call.request.id.transaction.as_u32() == tx
                            && call.request.id.sequence == *sequence,
                        "FINAL_CHAIN_NATIVE_LIFECYCLE_CALL_ORDER"
                    );
                    for read in &call.raw_reads {
                        let key = (read.address, read.key);
                        if let Some((phase, prior)) = known.get(&key) {
                            let consistent = if *phase == Some(tx) {
                                prior == &read.value
                            } else {
                                match (prior, &read.value) {
                                    (ConcreteRead::Present(a), ConcreteRead::Present(b)) => a == b,
                                    (
                                        ConcreteRead::Present(v),
                                        ConcreteRead::Absent | ConcreteRead::Tombstone,
                                    )
                                    | (
                                        ConcreteRead::Absent | ConcreteRead::Tombstone,
                                        ConcreteRead::Present(v),
                                    ) => v.is_empty(),
                                    _ => true,
                                }
                            };
                            ensure!(
                                consistent,
                                "FINAL_CHAIN_NATIVE_CONTEXT_RAW_READ_LINEAGE_MISMATCH"
                            );
                        } else if result.absent_accounts.contains(&read.address) {
                            ensure!(
                                matches!(read.value, ConcreteRead::Absent),
                                "FINAL_CHAIN_NATIVE_CONTEXT_RAW_READ_LINEAGE_MISMATCH"
                            );
                        }
                        known.insert(key, (Some(tx), read.value.clone()));
                    }
                    active = Some(next);
                }
                FinalChainLifecycleEvent::NativeObserved {
                    transaction,
                    sequence,
                } => {
                    let index = active.take().ok_or_else(|| {
                        anyhow::anyhow!("FINAL_CHAIN_NATIVE_LIFECYCLE_END_WITHOUT_START")
                    })?;
                    let call = &context.invocations[index];
                    ensure!(
                        *transaction == tx && call.request.id.sequence == *sequence,
                        "FINAL_CHAIN_NATIVE_LIFECYCLE_END_MISMATCH"
                    );
                    let mut touched = BTreeSet::new();
                    for mutation in &call.raw_mutations {
                        let key = (mutation.address, mutation.key);
                        ensure!(
                            (touched.contains(&key)
                                || call
                                    .raw_reads
                                    .iter()
                                    .any(|read| (read.address, read.key) == key))
                                && known
                                    .get(&key)
                                    .is_some_and(|(phase, value)| *phase == Some(tx)
                                        && value == &mutation.expected),
                            "FINAL_CHAIN_NATIVE_CONTEXT_RAW_EXPECTED_MISMATCH"
                        );
                        if !delta.values.contains_key(&key)
                            && !matches!(&mutation.expected, ConcreteRead::Present(v) if !v.is_empty())
                        {
                            delta.initially_absent.insert(key);
                        }
                        if matches!(&mutation.operation, FinalChainNativeRawOperation::Put(_)) {
                            delta.created.insert(key);
                        }
                        let value = match &mutation.operation {
                            FinalChainNativeRawOperation::Put(v) => v.as_bytes().to_vec(),
                            FinalChainNativeRawOperation::Delete => Vec::new(),
                        };
                        delta.erased.remove(&key);
                        pending
                            .entry(key)
                            .or_default()
                            .insert(*sequence, (ordinal, value.clone()));
                        known.insert(key, (Some(tx), ConcreteRead::Present(value.clone())));
                        delta.values.insert(key, value.clone());
                        result.final_values.insert(key, value);
                        touched.insert(key);
                    }
                    next += 1;
                    observed += 1;
                }
                FinalChainLifecycleEvent::TransactionEnd { observations } => {
                    ensure!(
                        active.is_none()
                            && *observations == observed
                            && ordinal + 1 == receipt.events.len(),
                        "FINAL_CHAIN_NATIVE_LIFECYCLE_TERMINAL"
                    );
                    ended = true;
                }
            }
        }
        ensure!(
            ended && active.is_none(),
            "FINAL_CHAIN_NATIVE_LIFECYCLE_UNSEALED"
        );
        result.transactions.insert(tx, delta);
    }
    ensure!(
        next == context.invocations.len(),
        "FINAL_CHAIN_NATIVE_LIFECYCLE_CALL_COUNT"
    );
    for read in &context.rewards.raw_reads {
        let key = (read.address, read.key);
        if let Some((_, prior)) = known.get(&key) {
            let consistent = match (prior, &read.value) {
                (ConcreteRead::Present(a), ConcreteRead::Present(b)) => a == b,
                (ConcreteRead::Present(v), ConcreteRead::Absent | ConcreteRead::Tombstone)
                | (ConcreteRead::Absent | ConcreteRead::Tombstone, ConcreteRead::Present(v)) => {
                    v.is_empty()
                }
                _ => true,
            };
            ensure!(
                consistent,
                "FINAL_CHAIN_NATIVE_CONTEXT_RAW_READ_LINEAGE_MISMATCH"
            );
        } else if result.absent_accounts.contains(&read.address) {
            ensure!(
                matches!(read.value, ConcreteRead::Absent),
                "FINAL_CHAIN_NATIVE_CONTEXT_RAW_READ_LINEAGE_MISMATCH"
            );
        }
        known.insert(key, (None, read.value.clone()));
    }
    for mutation in &context.rewards.raw_mutations {
        let key = (mutation.address, mutation.key);
        ensure!(
            known
                .get(&key)
                .is_some_and(|(phase, value)| phase.is_none() && value == &mutation.expected),
            "FINAL_CHAIN_NATIVE_CONTEXT_RAW_EXPECTED_MISMATCH"
        );
        let value = match &mutation.operation {
            FinalChainNativeRawOperation::Put(v) => v.as_bytes().to_vec(),
            FinalChainNativeRawOperation::Delete => Vec::new(),
        };
        known.insert(key, (None, ConcreteRead::Present(value.clone())));
        result.final_values.insert(key, value);
    }
    Ok(result)
}

fn validate_raw_lineage(context: &FinalChainNativeProjectionContext) -> Result<()> {
    fold_lifecycle_raw(context).map(|_| ())
}

#[cfg(feature = "synthetic-fixtures")]
#[derive(Debug)]
struct LineageOrigin {
    phase_index: usize,
    transaction: Option<u32>,
    sequence: Option<u64>,
    index: usize,
    operation: &'static str,
}

#[cfg(feature = "synthetic-fixtures")]
#[derive(Debug)]
struct LineageFailure {
    address: [u8; 20],
    key: rustaxa_types::concrete_state::ConcreteStorageKey,
    prior: rustaxa_types::concrete_state::ConcreteRead<Vec<u8>>,
    observed: rustaxa_types::concrete_state::ConcreteRead<Vec<u8>>,
    prior_origin: LineageOrigin,
    current_origin: LineageOrigin,
    same_phase: bool,
}

#[cfg(feature = "synthetic-fixtures")]
fn first_lineage_failure(
    context: &FinalChainNativeProjectionContext,
) -> Result<Option<LineageFailure>, ()> {
    use rustaxa_types::concrete_state::ConcreteRead;
    let mut known = BTreeMap::new();
    let phases = context
        .invocations
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            (
                index,
                Some(entry.request.id.transaction.as_u32()),
                Some(entry.request.id.sequence),
                entry.raw_reads.as_slice(),
                entry.raw_mutations.as_slice(),
            )
        })
        .chain(std::iter::once((
            context.invocations.len(),
            None,
            None,
            context.rewards.raw_reads.as_slice(),
            context.rewards.raw_mutations.as_slice(),
        )));
    let mut visited = 0usize;
    for (phase_index, transaction, sequence, reads, mutations) in phases {
        for (index, read) in reads.iter().enumerate() {
            visited += 1;
            if visited > 65536 {
                return Err(());
            }
            if matches!(&read.value, ConcreteRead::Present(v) if v.len() > 4096) {
                return Err(());
            }
            let key = (read.address, read.key);
            if let Some((prior, origin)) = known.get(&key) {
                let origin: &LineageOrigin = origin;
                let same_phase = origin.transaction == transaction;
                let consistent = if same_phase {
                    prior == &read.value
                } else {
                    match (prior, &read.value) {
                        (ConcreteRead::Present(left), ConcreteRead::Present(right)) => {
                            left == right
                        }
                        (
                            ConcreteRead::Present(value),
                            ConcreteRead::Absent | ConcreteRead::Tombstone,
                        )
                        | (
                            ConcreteRead::Absent | ConcreteRead::Tombstone,
                            ConcreteRead::Present(value),
                        ) => value.is_empty(),
                        _ => true,
                    }
                };
                if !consistent {
                    let (prior, prior_origin) = known.remove(&key).ok_or(())?;
                    return Ok(Some(LineageFailure {
                        address: read.address,
                        key: read.key,
                        prior,
                        observed: read.value.clone(),
                        prior_origin,
                        current_origin: LineageOrigin {
                            phase_index,
                            transaction,
                            sequence,
                            index,
                            operation: "read",
                        },
                        same_phase,
                    }));
                }
            }
            known.insert(
                key,
                (
                    read.value.clone(),
                    LineageOrigin {
                        phase_index,
                        transaction,
                        sequence,
                        index,
                        operation: "read",
                    },
                ),
            );
        }
        for (index, mutation) in mutations.iter().enumerate() {
            visited += 1;
            if visited > 65536 {
                return Err(());
            }
            let key = (mutation.address, mutation.key);
            if !known.get(&key).is_some_and(|(value, origin)| {
                let origin: &LineageOrigin = origin;
                origin.transaction == transaction && value == &mutation.expected
            }) {
                return Err(());
            }
            if matches!(&mutation.operation, FinalChainNativeRawOperation::Put(v) if v.as_bytes().len() > 4096)
            {
                return Err(());
            }
            let (value, operation) = match &mutation.operation {
                FinalChainNativeRawOperation::Put(value) => (value.as_bytes().to_vec(), "put"),
                FinalChainNativeRawOperation::Delete => (Vec::new(), "delete"),
            };
            known.insert(
                (mutation.address, mutation.key),
                (
                    ConcreteRead::Present(value),
                    LineageOrigin {
                        phase_index,
                        transaction,
                        sequence,
                        index,
                        operation,
                    },
                ),
            );
        }
    }
    Ok(None)
}

#[cfg(feature = "synthetic-fixtures")]
fn lineage_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(feature = "synthetic-fixtures")]
fn lineage_scope(
    context: &FinalChainNativeProjectionContext,
    request: [u8; 32],
    projection: &FinalChainConcreteStateProjection,
    enabled: bool,
) -> bool {
    enabled
        && context.request_id == request
        && lineage_hex(&request)
            == "2fda07600a98939e6c86070d81711d6b93c5417aface40e5e378c25a7073d388"
        && projection.generation == 5
        && projection.prior_state.period == 4
        && lineage_hex(&projection.prior_state.root)
            == "b934d70f36b020f10434857a9512ab00832370bfef875438cdb6e7ee58dd2b8e"
        && projection.post_transaction_state.period == 5
        && lineage_hex(&projection.post_transaction_state.root)
            == "5caad406f2d49cc61580c81a9cfaab801bb1147f654117af9a016b5a8e3f33bd"
}

#[cfg(feature = "synthetic-fixtures")]
fn dispatch_lineage_diagnostic(
    result: &Result<()>,
    context: &FinalChainNativeProjectionContext,
    request: [u8; 32],
    projection: &FinalChainConcreteStateProjection,
    enabled: bool,
    sink: &mut impl std::io::Write,
) {
    if result
        .as_ref()
        .is_err_and(|e| e.to_string() == "FINAL_CHAIN_NATIVE_CONTEXT_RAW_READ_LINEAGE_MISMATCH")
        && lineage_scope(context, request, projection, enabled)
    {
        emit_lineage_diagnostic(context, projection, sink);
    }
}

#[cfg(feature = "synthetic-fixtures")]
fn emit_lineage_diagnostic(
    context: &FinalChainNativeProjectionContext,
    projection: &FinalChainConcreteStateProjection,
    sink: &mut impl std::io::Write,
) {
    let result = first_lineage_failure(context);
    let overflow = result.is_err();
    let record = match result {
        Ok(Some(failure)) => {
            let current = &failure.current_origin;
            let prior = &failure.prior_origin;
            let kind = |index: usize| {
                if index == context.invocations.len() {
                    "rewards"
                } else {
                    "invocation"
                }
            };
            format!(
                "native_raw_lineage_first_comparison case_label=invalid_signature_a fixture_body_sha256_external_pin=b4d3532791ffb4270bf253ae36c537b10b40227db2405c5f4ba854ba2409b9b9 request_id={} projection_hash={} parent_period={} parent_root={} post_transaction_period={} post_transaction_root={} generation={} current_phase={} current_phase_index={} invocation_vector_index={:?} current_transaction={:?} current_sequence={:?} read_index={} address={:?} logical_key={:?} prior_value={:?} observed_value={:?} prior_phase={} prior_phase_index={} prior_transaction={:?} prior_sequence={:?} prior_index={} context_origin={} mutation_operation={} same_phase={} comparison_rule={} consistent=false provider_origin=NOT_RETAINED journal_origin=NOT_RETAINED provider_identity=NOT_RETAINED view_identity=NOT_RETAINED proof_status=NOT_RETAINED diagnostic_complete=false\n",
                lineage_hex(&context.request_id),
                lineage_hex(&context.projection_hash),
                projection.prior_state.period,
                lineage_hex(&projection.prior_state.root),
                projection.post_transaction_state.period,
                lineage_hex(&projection.post_transaction_state.root),
                projection.generation,
                kind(current.phase_index),
                current.phase_index,
                (current.phase_index < context.invocations.len()).then_some(current.phase_index),
                current.transaction,
                current.sequence,
                current.index,
                failure.address,
                failure.key,
                failure.prior,
                failure.observed,
                kind(prior.phase_index),
                prior.phase_index,
                prior.transaction,
                prior.sequence,
                prior.index,
                if prior.operation == "read" {
                    "read"
                } else {
                    "mutation"
                },
                prior.operation,
                failure.same_phase,
                if failure.same_phase {
                    "exact"
                } else {
                    "cross_phase"
                }
            )
        }
        _ => String::new(),
    };
    let complete = !record.is_empty() && record.len() <= 65536 - 128;
    if !complete {
        let marker = if overflow || record.len() > 65536 - 128 {
            b"native_raw_lineage_completion comparison_record_complete=false diagnostic_complete=false overflow=true writes_ok=true\n".as_slice()
        } else {
            b"native_raw_lineage_completion comparison_record_complete=false diagnostic_complete=false overflow=false writes_ok=true\n".as_slice()
        };
        if sink.write_all(marker).is_err() {
            let _ = sink.write_all(b"native_raw_lineage_completion comparison_record_complete=false diagnostic_complete=false overflow=false writes_ok=false\n");
        }
        return;
    }
    if sink.write_all(record.as_bytes()).is_err() || sink.write_all(b"native_raw_lineage_completion comparison_record_complete=true diagnostic_complete=false overflow=false writes_ok=true\n").is_err() {
        let _ = sink.write_all(b"native_raw_lineage_completion comparison_record_complete=false diagnostic_complete=false overflow=false writes_ok=false\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::concrete_state_projection::FinalChainConcreteInvocation;
    use crate::native_session::{FinalChainNativeInvocationId, FinalChainNativeValue};

    fn test_receipt(tx: u32, events: Vec<FinalChainLifecycleEvent>) -> FinalChainLifecycleReceipt {
        FinalChainLifecycleReceipt {
            transaction_position: tx,
            transaction_hash: [tx as u8; 32],
            token: u64::from(tx) + 1,
            event_count: events.len(),
            events,
        }
    }

    fn test_simple_lifecycle(context: &mut FinalChainNativeProjectionContext) {
        let mut grouped: BTreeMap<u32, Vec<FinalChainLifecycleEvent>> = BTreeMap::new();
        for call in &context.invocations {
            let transaction = call.request.id.transaction.as_u32();
            let sequence = call.request.id.sequence;
            grouped.entry(transaction).or_default().extend([
                FinalChainLifecycleEvent::NativeStarted {
                    transaction,
                    sequence,
                },
                FinalChainLifecycleEvent::NativeObserved {
                    transaction,
                    sequence,
                },
            ]);
        }
        context.lifecycle = grouped
            .into_iter()
            .map(|(tx, mut events)| {
                let observations = events.len() / 2;
                events.push(FinalChainLifecycleEvent::TransactionEnd { observations });
                test_receipt(tx, events)
            })
            .collect();
    }

    fn fixture() -> (
        FinalChainNativeProjectionContext,
        FinalChainConcreteStateProjection,
    ) {
        let request = FinalChainNativeRequest {
            id: FinalChainNativeInvocationId {
                transaction: 0_u32.into(),
                sequence: 0,
            },
            period: 1_u64.into(),
            depth: 1,
            kind: FinalChainNativeCallKind::Call,
            is_static: true,
            caller: [0x31; 20],
            contract: crate::final_chain::DPOS_CONTRACT_ADDRESS,
            state_address: crate::final_chain::DPOS_CONTRACT_ADDRESS,
            value: FinalChainNativeValue::new((BigUint::from(1_u8) << 264) + BigUint::from(7_u8)),
            input: vec![1, 2, 3, 4],
            supplied_gas: 20_000_u64.into(),
        };
        let mut projection = FinalChainConcreteStateProjection::default();
        projection.post_transaction_state.period = 1;
        projection.invocations.push(FinalChainConcreteInvocation {
            depth: request.depth,
            caller: request.caller,
            contract: request.contract,
            value: request.value.value().to_bytes_be(),
            input: request.input.clone(),
            supplied_gas: request.supplied_gas.as_u64(),
            ..Default::default()
        });
        let context = FinalChainNativeProjectionContext {
            request_id: [5; 32],
            projection_hash: concrete_state_bytes_digest(&encode_concrete_state_projection(
                &projection,
            )),
            lifecycle: vec![test_receipt(
                0,
                vec![
                    FinalChainLifecycleEvent::NativeStarted {
                        transaction: 0,
                        sequence: 0,
                    },
                    FinalChainLifecycleEvent::NativeObserved {
                        transaction: 0,
                        sequence: 0,
                    },
                    FinalChainLifecycleEvent::TransactionEnd { observations: 1 },
                ],
            )],
            rewards: FinalChainNativeRewardsContext::default(),
            invocations: vec![FinalChainNativeInvocationContext {
                request,
                accounts: vec![(
                    [0x31; 20],
                    FinalChainNativeAccount {
                        exists: true,
                        nonce: rustaxa_types::FinalChainNonce::from_u64(3),
                        balance: -(num_bigint::BigInt::from(1_u8) << 280_usize),
                    },
                )],
                raw_reads: Vec::new(),
                ordinary_mutations: Vec::new(),
                raw_mutations: Vec::new(),
            }],
        };
        (context, projection)
    }

    #[test]
    fn context_retains_full_values_and_inherited_static_call_facts() {
        let (context, mut projection) = fixture();
        context.validate_binding([5; 32], &projection).unwrap();
        // Binding is numeric, without narrowing or rejecting leading zero bytes.
        projection.invocations[0].value.insert(0, 0);
        let mut normalized = context;
        normalized.projection_hash =
            concrete_state_bytes_digest(&encode_concrete_state_projection(&projection));
        normalized.validate_binding([5; 32], &projection).unwrap();
        normalized.invocations[0].request.value = FinalChainNativeValue::new(7_u8.into());
        assert!(
            normalized
                .validate_binding([5; 32], &projection)
                .unwrap_err()
                .to_string()
                .contains("FRAME_MISMATCH")
        );
    }

    #[test]
    fn context_rejects_binding_context_and_account_integrity_mismatches() {
        let (context, projection) = fixture();
        assert!(
            context
                .validate_binding([6; 32], &projection)
                .unwrap_err()
                .to_string()
                .contains("REQUEST_MISMATCH")
        );
        let mut changed = projection.clone();
        changed.generation += 1;
        assert!(
            context
                .validate_binding([5; 32], &changed)
                .unwrap_err()
                .to_string()
                .contains("PROJECTION_MISMATCH")
        );
        for mutation in 0..8 {
            let mut invalid = context.clone();
            let entry = &mut invalid.invocations[0];
            let expected = match mutation {
                0 => {
                    entry.request.id.sequence = 1;
                    "FRAME_MISMATCH"
                }
                1 => {
                    entry.request.period = 2_u64.into();
                    "FRAME_MISMATCH"
                }
                2 => {
                    entry.request.state_address = [0; 20];
                    "ADDRESS_UNSUPPORTED"
                }
                3 => {
                    entry.request.kind = FinalChainNativeCallKind::DelegateCall;
                    "CALL_KIND_UNSUPPORTED"
                }
                4 => {
                    entry.accounts.push(entry.accounts[0].clone());
                    "DUPLICATE_ACCOUNT"
                }
                5 => {
                    entry.accounts[0].1.exists = false;
                    "ABSENT_ACCOUNT_NONZERO"
                }
                6 => {
                    invalid.invocations.clear();
                    "COUNT_MISMATCH"
                }
                _ => {
                    invalid.invocations.push(invalid.invocations[0].clone());
                    "COUNT_MISMATCH"
                }
            };
            assert!(
                invalid
                    .validate_binding([5; 32], &projection)
                    .unwrap_err()
                    .to_string()
                    .contains(expected)
            );
        }
    }

    #[test]
    fn staticcall_requires_effective_static_mode_and_zero_value() {
        let (mut context, mut projection) = fixture();
        context.invocations[0].request.kind = FinalChainNativeCallKind::StaticCall;
        projection.invocations[0].call_type = 3;
        context.projection_hash =
            concrete_state_bytes_digest(&encode_concrete_state_projection(&projection));
        assert!(
            context
                .validate_binding([5; 32], &projection)
                .unwrap_err()
                .to_string()
                .contains("STATIC_MISMATCH")
        );
        context.invocations[0].request.value = FinalChainNativeValue::default();
        projection.invocations[0].value.clear();
        context.projection_hash =
            concrete_state_bytes_digest(&encode_concrete_state_projection(&projection));
        context.validate_binding([5; 32], &projection).unwrap();
        context.invocations[0].request.is_static = false;
        assert!(
            context
                .validate_binding([5; 32], &projection)
                .unwrap_err()
                .to_string()
                .contains("STATIC_MISMATCH")
        );
    }
    #[cfg(feature = "synthetic-fixtures")]
    #[test]
    fn lineage_probe_dispatch_scope_and_result() {
        use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStorageKey};
        fn hex32(s: &str) -> [u8; 32] {
            let mut out = [0; 32];
            for (i, value) in out.iter_mut().enumerate() {
                *value = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
            }
            out
        }
        let (mut context, mut projection) = fixture();
        let request = hex32("2fda07600a98939e6c86070d81711d6b93c5417aface40e5e378c25a7073d388");
        context.request_id = request;
        projection.generation = 5;
        projection.prior_state.period = 4;
        projection.prior_state.root =
            hex32("b934d70f36b020f10434857a9512ab00832370bfef875438cdb6e7ee58dd2b8e");
        projection.post_transaction_state.period = 5;
        projection.post_transaction_state.root =
            hex32("5caad406f2d49cc61580c81a9cfaab801bb1147f654117af9a016b5a8e3f33bd");
        let address = crate::final_chain::DPOS_CONTRACT_ADDRESS;
        let key = ConcreteStorageKey([9; 32]);
        context.invocations[0].raw_reads = vec![FinalChainNativeRawRead {
            address,
            key,
            value: ConcreteRead::Present(vec![1]),
        }];
        context.rewards.raw_reads = vec![FinalChainNativeRawRead {
            address,
            key,
            value: ConcreteRead::Present(vec![2]),
        }];
        let error = validate_raw_lineage(&context);
        let before = error.as_ref().unwrap_err().to_string();
        let mut output = Vec::new();
        dispatch_lineage_diagnostic(&error, &context, request, &projection, true, &mut output);
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("comparison_record_complete=true")
        );
        assert_eq!(error.as_ref().unwrap_err().to_string(), before);
        let mut output = Vec::new();
        dispatch_lineage_diagnostic(&error, &context, request, &projection, false, &mut output);
        assert!(output.is_empty());
        for variant in 0..8 {
            let mut c = context.clone();
            let mut p = projection.clone();
            let mut r = request;
            match variant {
                0 => c.request_id[0] ^= 1,
                1 => r[0] ^= 1,
                2 => p.generation += 1,
                3 => p.prior_state.period += 1,
                4 => p.prior_state.root[0] ^= 1,
                5 => p.post_transaction_state.period += 1,
                6 => p.post_transaction_state.root[0] ^= 1,
                _ => {
                    c.request_id[0] ^= 1;
                    r[0] ^= 1;
                }
            }
            let mut output = Vec::new();
            dispatch_lineage_diagnostic(&error, &c, r, &p, true, &mut output);
            assert!(output.is_empty(), "variant {variant}");
        }
        for other in [Ok(()), Err(anyhow::anyhow!("different error"))] {
            let mut output = Vec::new();
            dispatch_lineage_diagnostic(&other, &context, request, &projection, true, &mut output);
            assert!(output.is_empty());
        }
    }

    #[cfg(feature = "synthetic-fixtures")]
    #[test]
    fn lineage_probe_scan_boundary_and_first_nonzero_indices() {
        use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStorageKey};
        let (mut context, _) = fixture();
        let address = crate::final_chain::DPOS_CONTRACT_ADDRESS;
        let first = ConcreteStorageKey([8; 32]);
        let later = ConcreteStorageKey([9; 32]);
        let read = |key, value| FinalChainNativeRawRead {
            address,
            key,
            value: ConcreteRead::Present(vec![value]),
        };
        context.invocations[0].request.id.transaction = 7_u32.into();
        context.invocations[0].request.id.sequence = 11;
        context.invocations[0].raw_reads = vec![
            read(ConcreteStorageKey([1; 32]), 0),
            read(first, 1),
            read(later, 3),
        ];
        context.invocations[0].raw_mutations.clear();
        let mut next = context.invocations[0].clone();
        next.request.id.sequence = 12;
        next.raw_reads = vec![
            read(ConcreteStorageKey([1; 32]), 0),
            read(first, 2),
            read(later, 4),
        ];
        context.invocations.push(next);
        let f = first_lineage_failure(&context).unwrap().unwrap();
        assert_eq!(
            (f.address, f.key, f.prior, f.observed),
            (
                address,
                first,
                ConcreteRead::Present(vec![1]),
                ConcreteRead::Present(vec![2])
            )
        );
        assert_eq!(
            (
                f.prior_origin.phase_index,
                f.prior_origin.transaction,
                f.prior_origin.sequence,
                f.prior_origin.index
            ),
            (0, Some(7), Some(11), 1)
        );
        assert_eq!(
            (
                f.current_origin.phase_index,
                f.current_origin.transaction,
                f.current_origin.sequence,
                f.current_origin.index
            ),
            (1, Some(7), Some(12), 1)
        );
        context.invocations.truncate(1);
        context.invocations[0].raw_reads = vec![read(first, 1); 65535];
        context.rewards.raw_reads = vec![read(first, 2)];
        let f = first_lineage_failure(&context).unwrap().unwrap();
        assert_eq!(f.current_origin.index, 0);
        context.invocations[0].raw_reads.push(read(first, 1));
        assert!(first_lineage_failure(&context).is_err());
        let (_, projection) = fixture();
        let mut output = Vec::new();
        emit_lineage_diagnostic(&context, &projection, &mut output);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("overflow=true"));
        assert!(!output.contains("comparison_record_complete=true"));
    }

    #[cfg(feature = "synthetic-fixtures")]
    #[test]
    fn lineage_probe_finds_first_read_and_delete_origins() {
        use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStorageKey};
        let (mut context, projection) = fixture();
        let address = crate::final_chain::DPOS_CONTRACT_ADDRESS;
        let key = ConcreteStorageKey([8; 32]);
        context.invocations[0].raw_reads = vec![FinalChainNativeRawRead {
            address,
            key,
            value: ConcreteRead::Present(vec![1]),
        }];
        context.invocations[0].raw_mutations.clear();
        let mut next = context.invocations[0].clone();
        next.request.id.sequence = 1;
        next.raw_reads[0].value = ConcreteRead::Present(vec![2]);
        context.invocations.push(next);
        let failure = first_lineage_failure(&context).unwrap().unwrap();
        assert_eq!(
            (
                failure.prior_origin.operation,
                failure.prior_origin.phase_index,
                failure.prior_origin.index
            ),
            ("read", 0, 0)
        );
        assert_eq!(
            (
                failure.current_origin.phase_index,
                failure.current_origin.index,
                failure.same_phase
            ),
            (1, 0, true)
        );
        assert_eq!(failure.prior, ConcreteRead::Present(vec![1]));
        context.invocations[0]
            .raw_mutations
            .push(FinalChainNativeRawMutation {
                address,
                key,
                expected: ConcreteRead::Present(vec![1]),
                operation: FinalChainNativeRawOperation::Delete,
            });
        let failure = first_lineage_failure(&context).unwrap().unwrap();
        assert_eq!(failure.prior_origin.operation, "delete");
        assert_eq!(failure.prior, ConcreteRead::Present(vec![]));
        context.invocations.truncate(1);
        context.rewards.raw_reads = vec![FinalChainNativeRawRead {
            address,
            key,
            value: ConcreteRead::Present(vec![2]),
        }];
        let failure = first_lineage_failure(&context).unwrap().unwrap();
        assert_eq!(failure.current_origin.phase_index, 1);
        assert_eq!(failure.current_origin.transaction, None);
        assert!(!failure.same_phase);
        let mut output = Vec::new();
        emit_lineage_diagnostic(&context, &projection, &mut output);
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("comparison_record_complete=true")
        );
        context.rewards.raw_reads[0].value = ConcreteRead::Present(vec![]);
        assert!(first_lineage_failure(&context).unwrap().is_none());
        context.rewards.raw_reads[0].value = ConcreteRead::Present(vec![1; 4097]);
        assert!(first_lineage_failure(&context).is_err());
        let mut output = Vec::new();
        emit_lineage_diagnostic(&context, &projection, &mut output);
        assert!(String::from_utf8(output).unwrap().contains("overflow=true"));
        struct Fail;
        impl std::io::Write for Fail {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("fail"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        emit_lineage_diagnostic(&context, &projection, &mut Fail);
        assert!(validate_raw_lineage(&context).is_err());
    }

    #[test]
    fn raw_lineage_preserves_journal_and_prepared_view_classifications() {
        use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStorageKey};
        let (mut context, _) = fixture();
        let address = crate::final_chain::DPOS_CONTRACT_ADDRESS;
        let key = ConcreteStorageKey([8; 32]);
        context.invocations[0].raw_reads = vec![FinalChainNativeRawRead {
            address,
            key,
            value: ConcreteRead::Present(vec![1]),
        }];
        context.invocations[0].raw_mutations = vec![FinalChainNativeRawMutation {
            address,
            key,
            expected: ConcreteRead::Present(vec![1]),
            operation: FinalChainNativeRawOperation::Delete,
        }];
        let mut second = context.invocations[0].clone();
        second.request.id.sequence = 1;
        second.raw_reads[0].value = ConcreteRead::Present(Vec::new());
        second.raw_mutations.clear();
        context.invocations.push(second);
        test_simple_lifecycle(&mut context);
        validate_raw_lineage(&context).unwrap();
        context.invocations[1].raw_reads[0].value = ConcreteRead::Tombstone;
        assert!(
            validate_raw_lineage(&context)
                .unwrap_err()
                .to_string()
                .contains("READ_LINEAGE_MISMATCH")
        );
        context.invocations[1].request.id.transaction = 1_u32.into();
        test_simple_lifecycle(&mut context);
        validate_raw_lineage(&context).unwrap();
        context.invocations[1].raw_reads[0].value = ConcreteRead::Absent;
        validate_raw_lineage(&context).unwrap();
        context.invocations[1].raw_reads[0].value = ConcreteRead::Present(vec![2]);
        assert!(
            validate_raw_lineage(&context)
                .unwrap_err()
                .to_string()
                .contains("READ_LINEAGE_MISMATCH")
        );
        context.invocations.truncate(1);
        test_simple_lifecycle(&mut context);
        context.invocations[0].raw_mutations[0].expected = ConcreteRead::Tombstone;
        assert!(
            validate_raw_lineage(&context)
                .unwrap_err()
                .to_string()
                .contains("RAW_EXPECTED_MISMATCH")
        );
    }
    #[test]
    fn native_failure_cannot_be_reclassified_as_parent_rollback() {
        let (mut context, mut projection) = fixture();
        projection.invocations[0].error = "Method is not payable".to_owned();
        projection.invocations[0].disposition =
            crate::concrete_state_projection::FINAL_CHAIN_CONCRETE_INVOCATION_PARENT_FRAME_REVERTED;
        context.projection_hash =
            concrete_state_bytes_digest(&encode_concrete_state_projection(&projection));
        assert!(
            context
                .validate_binding([5; 32], &projection)
                .unwrap_err()
                .to_string()
                .contains("DISPOSITION_MISMATCH")
        );
        projection.invocations[0].disposition =
            crate::concrete_state_projection::FINAL_CHAIN_CONCRETE_INVOCATION_OWN_FRAME_REVERTED;
        context.projection_hash =
            concrete_state_bytes_digest(&encode_concrete_state_projection(&projection));
        context.validate_binding([5; 32], &projection).unwrap();
    }
    #[test]
    fn lifecycle_reset_removes_only_the_created_account_raw_effects() {
        use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStorageKey};
        let (mut context, _) = fixture();
        let address = crate::final_chain::DPOS_CONTRACT_ADDRESS;
        let other = crate::final_chain::SLASHING_CONTRACT_ADDRESS;
        let key = ConcreteStorageKey([7; 32]);
        context.invocations[0].raw_reads = vec![
            FinalChainNativeRawRead {
                address,
                key,
                value: ConcreteRead::Absent,
            },
            FinalChainNativeRawRead {
                address: other,
                key,
                value: ConcreteRead::Absent,
            },
        ];
        context.invocations[0].raw_mutations = [address, other]
            .into_iter()
            .map(|address| FinalChainNativeRawMutation {
                address,
                key,
                expected: ConcreteRead::Absent,
                operation: FinalChainNativeRawOperation::Put(
                    crate::native_session::FinalChainNativeRawValue::new(vec![1]).unwrap(),
                ),
            })
            .collect();
        use FinalChainLifecycleEvent::*;
        let events = vec![
            FrameOpened { parent: None },
            NativeStarted {
                transaction: 0,
                sequence: 0,
            },
            AccountCreated {
                address,
                frame: Some(0),
                saved_absent: true,
            },
            NativeObserved {
                transaction: 0,
                sequence: 0,
            },
            AccountReset {
                address,
                creation: 2,
                rollback_frame: 0,
            },
            TransactionEnd { observations: 1 },
        ];
        context.lifecycle = vec![test_receipt(0, events)];
        let fold = fold_lifecycle_raw(&context).unwrap();
        assert!(!fold.final_values.contains_key(&(address, key)));
        assert_eq!(fold.final_values.get(&(other, key)), Some(&vec![1]));
        assert!(fold.absent_accounts.contains(&address));
        assert!(!fold.transactions[&0].values.contains_key(&(address, key)));
        let witness = &fold.transactions[&0].erased[&(address, key)][0];
        assert_eq!(
            (witness.sequence, witness.observed, witness.reset),
            (0, 3, 4)
        );
        assert_eq!(witness.value, vec![1]);
        assert!(!fold.transactions[&0].erased.contains_key(&(other, key)));
        let mut second = context.invocations[0].clone();
        second.request.id.sequence = 1;
        second.raw_reads.retain(|read| read.address == address);
        second
            .raw_mutations
            .retain(|mutation| mutation.address == address);
        context.invocations.push(second);
        context.lifecycle[0].events.pop();
        context.lifecycle[0].events.extend([
            NativeStarted {
                transaction: 0,
                sequence: 1,
            },
            NativeObserved {
                transaction: 0,
                sequence: 1,
            },
            TransactionEnd { observations: 2 },
        ]);
        context.lifecycle[0].event_count = context.lifecycle[0].events.len();
        let fold = fold_lifecycle_raw(&context).unwrap();
        assert!(!fold.transactions[&0].erased.contains_key(&(address, key)));
        assert_eq!(
            fold.transactions[&0].values.get(&(address, key)),
            Some(&vec![1])
        );
        context.invocations.pop();
        context.lifecycle[0].events.truncate(5);
        context.lifecycle[0]
            .events
            .push(TransactionEnd { observations: 1 });
        context.lifecycle[0].event_count = context.lifecycle[0].events.len();
        context.lifecycle[0].events.remove(4);
        context.lifecycle[0].event_count -= 1;
        assert_eq!(
            fold_lifecycle_raw(&context)
                .unwrap()
                .final_values
                .get(&(address, key)),
            Some(&vec![1])
        );
    }

    #[test]
    fn lifecycle_intervals_and_expected_values_are_checked() {
        use FinalChainLifecycleEvent::*;
        use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStorageKey};
        let (mut context, _) = fixture();
        let address = crate::final_chain::DPOS_CONTRACT_ADDRESS;
        let key = ConcreteStorageKey([8; 32]);
        context.invocations[0].raw_reads = vec![FinalChainNativeRawRead {
            address,
            key,
            value: ConcreteRead::Absent,
        }];
        context.invocations[0].raw_mutations = vec![
            FinalChainNativeRawMutation {
                address,
                key,
                expected: ConcreteRead::Absent,
                operation: FinalChainNativeRawOperation::Put(
                    crate::native_session::FinalChainNativeRawValue::new(vec![1]).unwrap(),
                ),
            },
            FinalChainNativeRawMutation {
                address,
                key,
                expected: ConcreteRead::Present(vec![1]),
                operation: FinalChainNativeRawOperation::Delete,
            },
        ];
        assert!(fold_lifecycle_raw(&context).is_ok());
        context.invocations[0].raw_mutations[1].expected = ConcreteRead::Absent;
        assert!(fold_lifecycle_raw(&context).is_err());
        context.invocations[0].raw_mutations[1].expected = ConcreteRead::Present(vec![1]);
        for bad in [
            vec![
                NativeStarted {
                    transaction: 0,
                    sequence: 0,
                },
                NativeStarted {
                    transaction: 0,
                    sequence: 0,
                },
                TransactionEnd { observations: 0 },
            ],
            vec![
                NativeStarted {
                    transaction: 0,
                    sequence: 0,
                },
                NativeObserved {
                    transaction: 0,
                    sequence: 1,
                },
                TransactionEnd { observations: 1 },
            ],
            vec![
                NativeStarted {
                    transaction: 0,
                    sequence: 0,
                },
                TransactionEnd { observations: 0 },
            ],
            vec![
                FrameOpened { parent: None },
                AccountCreated {
                    address,
                    frame: Some(0),
                    saved_absent: true,
                },
                NativeStarted {
                    transaction: 0,
                    sequence: 0,
                },
                AccountReset {
                    address,
                    creation: 1,
                    rollback_frame: 0,
                },
                NativeObserved {
                    transaction: 0,
                    sequence: 0,
                },
                TransactionEnd { observations: 1 },
            ],
        ] {
            context.lifecycle = vec![test_receipt(0, bad)];
            assert!(fold_lifecycle_raw(&context).is_err());
        }
    }

    #[test]
    fn lifecycle_authority_rejects_changed_missing_duplicate_and_stale_claims() {
        let (context, _) = fixture();
        let authority = FinalChainLifecycleAuthority {
            request_id: [5; 32],
            state_api_epoch: 3,
            projection_hash: [9; 32],
            receipts: context.lifecycle.clone(),
        };
        let tx = [(0, [0; 32])];
        let check = |claims: &FinalChainNativeProjectionContext,
                     expected: &FinalChainLifecycleAuthority| {
            validate_lifecycle_authority(claims, expected, [5; 32], 3, [9; 32], &tx)
        };
        assert!(check(&context, &authority).is_ok());
        for change in 0..5 {
            let mut claims = context.clone();
            match change {
                0 => claims.lifecycle.clear(),
                1 => claims.lifecycle.push(claims.lifecycle[0].clone()),
                2 => claims.lifecycle[0].token += 1,
                3 => claims.lifecycle[0].events.swap(0, 1),
                _ => claims.lifecycle[0].events.pop().map(|_| ()).unwrap(),
            }
            assert!(check(&claims, &authority).is_err());
        }
        let mut stale = authority.clone();
        stale.state_api_epoch += 1;
        assert!(check(&context, &stale).is_err());
        assert!(
            validate_lifecycle_authority(
                &context,
                &authority,
                [5; 32],
                3,
                [9; 32],
                &[(0, [1; 32])]
            )
            .is_err()
        );
    }
    #[test]
    fn reset_then_recreate_and_multiple_calls_keep_only_later_effects() {
        use FinalChainLifecycleEvent::*;
        use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStorageKey};
        let (mut context, _) = fixture();
        let address = crate::final_chain::DPOS_CONTRACT_ADDRESS;
        let key = ConcreteStorageKey([4; 32]);
        context.invocations[0].raw_reads = vec![FinalChainNativeRawRead {
            address,
            key,
            value: ConcreteRead::Absent,
        }];
        context.invocations[0].raw_mutations = vec![FinalChainNativeRawMutation {
            address,
            key,
            expected: ConcreteRead::Absent,
            operation: FinalChainNativeRawOperation::Put(
                crate::native_session::FinalChainNativeRawValue::new(vec![1]).unwrap(),
            ),
        }];
        let mut second = context.invocations[0].clone();
        second.request.id.sequence = 1;
        second.raw_mutations[0].operation = FinalChainNativeRawOperation::Put(
            crate::native_session::FinalChainNativeRawValue::new(vec![2]).unwrap(),
        );
        context.invocations.push(second);
        context.lifecycle = vec![test_receipt(
            0,
            vec![
                FrameOpened { parent: None },
                NativeStarted {
                    transaction: 0,
                    sequence: 0,
                },
                AccountCreated {
                    address,
                    frame: Some(0),
                    saved_absent: true,
                },
                NativeObserved {
                    transaction: 0,
                    sequence: 0,
                },
                AccountReset {
                    address,
                    creation: 2,
                    rollback_frame: 0,
                },
                NativeStarted {
                    transaction: 0,
                    sequence: 1,
                },
                AccountCreated {
                    address,
                    frame: Some(0),
                    saved_absent: true,
                },
                NativeObserved {
                    transaction: 0,
                    sequence: 1,
                },
                TransactionEnd { observations: 2 },
            ],
        )];
        let fold = fold_lifecycle_raw(&context).unwrap();
        assert_eq!(fold.final_values.get(&(address, key)), Some(&vec![2]));
        assert_eq!(
            fold.transactions[&0].values.get(&(address, key)),
            Some(&vec![2])
        );
        context.invocations[1].raw_reads[0].value = ConcreteRead::Present(vec![1]);
        assert!(fold_lifecycle_raw(&context).is_err());
    }

    #[test]
    fn a_creation_without_reset_does_not_clear_a_previous_native_write() {
        use FinalChainLifecycleEvent::*;
        use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStorageKey};
        let (mut context, _) = fixture();
        let address = crate::final_chain::DPOS_CONTRACT_ADDRESS;
        let key = ConcreteStorageKey([6; 32]);
        context.invocations[0].raw_reads = vec![FinalChainNativeRawRead {
            address,
            key,
            value: ConcreteRead::Absent,
        }];
        context.invocations[0].raw_mutations = vec![FinalChainNativeRawMutation {
            address,
            key,
            expected: ConcreteRead::Absent,
            operation: FinalChainNativeRawOperation::Put(
                crate::native_session::FinalChainNativeRawValue::new(vec![1]).unwrap(),
            ),
        }];
        let mut second = context.invocations[0].clone();
        second.request.id.sequence = 1;
        second.raw_reads[0].value = ConcreteRead::Present(vec![1]);
        second.raw_mutations.clear();
        context.invocations.push(second);
        context.lifecycle = vec![test_receipt(
            0,
            vec![
                FrameOpened { parent: None },
                NativeStarted {
                    transaction: 0,
                    sequence: 0,
                },
                NativeObserved {
                    transaction: 0,
                    sequence: 0,
                },
                NativeStarted {
                    transaction: 0,
                    sequence: 1,
                },
                AccountCreated {
                    address,
                    frame: Some(0),
                    saved_absent: true,
                },
                NativeObserved {
                    transaction: 0,
                    sequence: 1,
                },
                TransactionEnd { observations: 2 },
            ],
        )];
        assert_eq!(
            fold_lifecycle_raw(&context)
                .unwrap()
                .final_values
                .get(&(address, key)),
            Some(&vec![1])
        );
    }
}
