//! Disposable default structured tracing over preceding committed state.
//!
//! The pinned Go `TraceRunner` executes prerequisite and target transactions in
//! one `BlockState` rooted at `max(execution_period - 1, 0)`. It does not commit
//! or reset that state between calls. This module preserves that lifecycle with
//! one dropped [`ExecutionJournal`]: dirty/original storage, refunds, logs,
//! transient values, account lifecycle and supplied nonces remain sequence local.
//! It returns only serialized structured results and the authenticated identity
//! used to start the sequence; it exposes no writes or publication authority.

use rustaxa_types::concrete_state::{
    ConcreteAccountRecord, ConcreteRead, ConcreteReadError, ConcreteStateIdentity,
    ConcreteStateRead, ConcreteStorageKey, execution::ConcreteExecutionRead,
};

use crate::{
    contracts::{
        BlockHashRead, CodeExecutionStatus, ExecutionBlockContext, ExecutionTransaction,
        ExecutionTransactionKind, NativeExecutionPort, NativePortError, TransactionExecutionResult,
    },
    driver::{
        ExecutionDriverError, NativeAddressClassifier, PeriodConsensusSequence,
        execute_top_level_call, execute_top_level_call_with_native,
        execute_top_level_call_with_trace, execute_top_level_create,
        execute_top_level_create_with_native, execute_top_level_create_with_trace,
    },
    envelope::EnvelopeRules,
    journal::ExecutionJournal,
    profile::TaraxaProfile,
    structured_trace::{
        StructuredTraceResult, StructuredTraceSerializationError, serialize_structured_results,
    },
    trace::{TraceCollector, TraceEvent},
};

/// Position of a transaction that aborted a disposable trace sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceSequenceStage {
    /// Unobserved prerequisite transaction.
    Prefix,
    /// Target transaction with a fresh structured collector.
    Target,
}

/// Failure to establish, execute, or serialize a structured trace sequence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StructuredTraceRunnerError {
    /// Request or prefix outcome is outside the measured P0 ordinary-prefix profile.
    ClaimAllPrefixScope,
    /// The supplied reader is not the exact preceding state selected by Go.
    /// No account, storage, code, or block-hash read has occurred.
    StatePeriodMismatch {
        /// Actual fixed reader identity, including its authenticated root.
        state: ConcreteStateIdentity,
        /// Required `max(execution_period - 1, 0)` state period.
        expected: rustaxa_types::FinalChainBlockNumber,
        /// Execution block period used to derive `expected`.
        execution: rustaxa_types::FinalChainBlockNumber,
    },
    /// Creating the private native port failed before sequence execution.
    NativeSession(NativePortError),
    /// One sequence transaction reached an infrastructure or unsupported path.
    Execution {
        /// Whether the transaction was a prerequisite or target.
        stage: TraceSequenceStage,
        /// Zero-based index within the corresponding input slice.
        index: usize,
        /// Exact existing driver error.
        error: ExecutionDriverError,
    },
    /// Observed facts could not form the qualified structured JSON shape.
    Serialization(StructuredTraceSerializationError),
}

impl std::fmt::Display for StructuredTraceRunnerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "structured trace runner: {self:?}")
    }
}

impl std::error::Error for StructuredTraceRunnerError {}

/// Default structured-trace bytes bound to the committed state actually used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructuredTraceRun {
    /// Fixed preceding period/root from which the disposable sequence began.
    pub state: ConcreteStateIdentity,
    /// Exact JSON array returned by the bounded structured serializer.
    pub output: Vec<u8>,
}

struct BorrowedCommitted<'a, R: ?Sized>(&'a R);

impl<R: ConcreteStateRead + ?Sized> ConcreteStateRead for BorrowedCommitted<'_, R> {
    fn identity(&self) -> ConcreteStateIdentity {
        self.0.identity()
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        self.0.account(address)
    }

    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.0.storage(address, key)
    }

    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.0.code(hash)
    }
}

struct OwnedStructuredResult {
    gas_used: u64,
    failed: bool,
    return_value: Vec<u8>,
    events: Vec<TraceEvent>,
}

/// Trace-owner proof of the exact successful P0 ordinary prefix.
///
/// Only [`run_structured_claim_all_prefix_trace`] can create this proof. The
/// factory borrows it after prefix execution and cannot retain it in its port.
/// The trusted adapter must associate its full concrete source and configuration
/// with the native semantic history; this proof does not authenticate that history.
pub struct ClaimAllPrefixTraceAuthority {
    state: ConcreteStateIdentity,
    block: ExecutionBlockContext,
    target: ExecutionTransaction,
}

impl ClaimAllPrefixTraceAuthority {
    /// Complete source identity from which the retained journal began.
    pub fn state(&self) -> ConcreteStateIdentity {
        self.state
    }
    /// Original execution block, separate from source period zero.
    pub fn block(&self) -> &ExecutionBlockContext {
        &self.block
    }
    /// Exact original target, including supplied wide nonce and position one.
    pub fn target(&self) -> &ExecutionTransaction {
        &self.target
    }
}

/// Runs only the measured source0/execution1 ordinary-prefix claim-all trace.
///
/// The single prefix is a successful code-free non-native self CALL with
/// nonce2^512, gas21000, zero price/value and empty input. The sole target is
/// position1 claimAll with nonce2^512+1 and gas200000. Both use one disposable
/// journal without reset. The native factory runs only after prefix success;
/// no native port or sequence exists during the prefix. Target native sequence
/// begins at zero. Wrong shapes/outcomes return `ClaimAllPrefixScope`; read and
/// driver errors retain Prefix0/Target0, and factory errors use NativeSession.
/// No prefix JSON, write set, publication authority or proof escapes.
#[allow(clippy::too_many_arguments)]
pub fn run_structured_claim_all_prefix_trace<
    R: ConcreteStateRead + ?Sized,
    B: BlockHashRead,
    A: NativeAddressClassifier,
    C: NativeAddressClassifier,
    P: NativeExecutionPort,
    F: for<'proof> FnOnce(&'proof ClaimAllPrefixTraceAuthority) -> Result<P, NativePortError>,
>(
    reader: &R,
    block_hashes: &B,
    all_native_addresses: &A,
    consensus_native_addresses: &C,
    native_factory: F,
    block: &ExecutionBlockContext,
    prefix: &[ExecutionTransaction],
    targets: &[ExecutionTransaction],
    envelope_rules: EnvelopeRules,
    profile: TaraxaProfile,
) -> Result<StructuredTraceRun, StructuredTraceRunnerError> {
    use crate::profile::TaraxaPhase;
    use num_bigint::BigUint;
    let state = reader.identity();
    let expected =
        rustaxa_types::FinalChainBlockNumber::new(block.period.as_u64().saturating_sub(1));
    if state.period != expected {
        return Err(StructuredTraceRunnerError::StatePeriodMismatch {
            state,
            expected,
            execution: block.period,
        });
    }
    let scope = StructuredTraceRunnerError::ClaimAllPrefixScope;
    let ([prior], [target]) = (prefix, targets) else {
        return Err(scope);
    };
    let nonce = BigUint::from(1u8) << 512usize;
    let mut dpos = [0; 20];
    dpos[19] = 0xfe;
    if block.period.as_u64() != 1
        || !envelope_rules.cornus
        || profile.phase() != TaraxaPhase::Ficus
        || prior.kind != ExecutionTransactionKind::Call
        || prior.position.as_u32() != 0
        || prior.receiver != Some(prior.sender)
        || prior.gas_limit.as_u64() != 21000
        || prior.gas_price.value() != &BigUint::default()
        || prior.value.value() != &BigUint::default()
        || !prior.input.is_empty()
        || BigUint::from_bytes_be(&prior.nonce.to_bytes()) != nonce
        || all_native_addresses.is_native_address(block.period, prior.sender)
        || consensus_native_addresses.is_native_address(block.period, prior.sender)
        || target.kind != ExecutionTransactionKind::Call
        || target.position.as_u32() != 1
        || target.sender != prior.sender
        || target.receiver != Some(dpos)
        || target.gas_limit.as_u64() != 200000
        || target.gas_price.value() != &BigUint::default()
        || target.value.value() != &BigUint::default()
        || target.input != [0x0b, 0x83, 0xa7, 0x27]
        || BigUint::from_bytes_be(&target.nonce.to_bytes()) != &nonce + 1u8
        || !consensus_native_addresses.is_native_address(block.period, dpos)
    {
        return Err(scope);
    }
    let prefix_error = |error| StructuredTraceRunnerError::Execution {
        stage: TraceSequenceStage::Prefix,
        index: 0,
        error,
    };
    let mut journal = ExecutionJournal::new(BorrowedCommitted(reader));
    let before = journal
        .account_metadata(prior.sender)
        .map_err(|e| prefix_error(ExecutionDriverError::Journal(e)))?;
    if !before.exists
        || !journal
            .account_code(prior.sender)
            .map_err(|e| prefix_error(ExecutionDriverError::Journal(e)))?
            .is_empty()
    {
        return Err(scope);
    }
    let result = execute_top_level_call(
        &mut journal,
        block_hashes,
        all_native_addresses,
        block,
        prior,
        envelope_rules,
        profile,
    )
    .map_err(prefix_error)?;
    if !matches!(result, TransactionExecutionResult::Executed(ref out)
        if out.status == CodeExecutionStatus::Success && out.gas_used.as_u64() == 21000
        && out.output.is_empty() && out.logs.is_empty())
    {
        return Err(scope);
    }
    let after = journal
        .account_metadata(prior.sender)
        .map_err(|e| prefix_error(ExecutionDriverError::Journal(e)))?;
    if !after.exists
        || after.nonce != prior.nonce.next()
        || after.balance != before.balance
        || !journal
            .account_code(prior.sender)
            .map_err(|e| prefix_error(ExecutionDriverError::Journal(e)))?
            .is_empty()
    {
        return Err(scope);
    }
    let authority = ClaimAllPrefixTraceAuthority {
        state,
        block: block.clone(),
        target: target.clone(),
    };
    let mut port = native_factory(&authority).map_err(StructuredTraceRunnerError::NativeSession)?;
    let mut sequence = PeriodConsensusSequence::new(block.period);
    let execution = execute_top_level_call_with_native(
        &mut journal,
        block_hashes,
        all_native_addresses,
        consensus_native_addresses,
        &mut port,
        &mut sequence,
        block,
        target,
        envelope_rules,
        profile,
    )
    .map_err(|error| StructuredTraceRunnerError::Execution {
        stage: TraceSequenceStage::Target,
        index: 0,
        error,
    })?;
    let result = owned_result(execution, TraceCollector::default());
    let output = serialize_structured_results(&[StructuredTraceResult {
        gas_used: result.gas_used,
        failed: result.failed,
        return_value: &result.return_value,
        events: &result.events,
    }])
    .map_err(StructuredTraceRunnerError::Serialization)?;
    Ok(StructuredTraceRun { state, output })
}

/// Runs one bounded default structured TraceRunner sequence.
///
/// `reader` must identify the committed state immediately preceding `block`,
/// except that execution period zero also selects state period zero. `prefix`
/// transactions execute first without observers. `targets` then execute in order
/// with one fresh collector apiece. Every transaction retains its supplied nonce;
/// this function does not apply the separate DryRunner nonce-replacement policy.
/// Consensus and code failures become ordinary JSON result rows for targets and
/// do not abort later targets. Prefix outcomes are intentionally discarded.
///
/// One journal is retained without [`ExecutionJournal::settle_transaction`]
/// across the whole sequence, matching Go's no-op `CommitTransaction` boundary.
/// The journal is always dropped before return, including on error. Direct
/// addresses selected by `native_addresses`, nested target calls, and trace fact
/// paths outside the reviewed driver boundary fail explicitly. This function
/// supports only the default structured mode; it does not route native calls,
/// implement OpenEthereum modes, open historical storage, or replay bytecode to
/// infer missing facts.
#[allow(clippy::too_many_arguments)]
pub fn run_structured_trace<
    R: ConcreteStateRead + ?Sized,
    B: BlockHashRead,
    N: NativeAddressClassifier,
>(
    reader: &R,
    block_hashes: &B,
    native_addresses: &N,
    block: &ExecutionBlockContext,
    prefix: &[ExecutionTransaction],
    targets: &[ExecutionTransaction],
    envelope_rules: EnvelopeRules,
    profile: TaraxaProfile,
) -> Result<StructuredTraceRun, StructuredTraceRunnerError> {
    let state = reader.identity();
    let expected =
        rustaxa_types::FinalChainBlockNumber::new(block.period.as_u64().saturating_sub(1));
    if state.period != expected {
        return Err(StructuredTraceRunnerError::StatePeriodMismatch {
            state,
            expected,
            execution: block.period,
        });
    }

    let mut journal = ExecutionJournal::new(BorrowedCommitted(reader));
    for (index, transaction) in prefix.iter().enumerate() {
        execute_untraced(
            &mut journal,
            block_hashes,
            native_addresses,
            block,
            transaction,
            envelope_rules,
            profile,
        )
        .map_err(|error| StructuredTraceRunnerError::Execution {
            stage: TraceSequenceStage::Prefix,
            index,
            error,
        })?;
    }

    let mut observed = Vec::with_capacity(targets.len());
    for (index, transaction) in targets.iter().enumerate() {
        let mut collector = TraceCollector::default();
        let execution = execute_traced(
            &mut journal,
            block_hashes,
            native_addresses,
            block,
            transaction,
            envelope_rules,
            profile,
            &mut collector,
        )
        .map_err(|error| StructuredTraceRunnerError::Execution {
            stage: TraceSequenceStage::Target,
            index,
            error,
        })?;
        observed.push(owned_result(execution, collector));
    }

    let supplied = observed
        .iter()
        .map(|result| StructuredTraceResult {
            gas_used: result.gas_used,
            failed: result.failed,
            return_value: &result.return_value,
            events: &result.events,
        })
        .collect::<Vec<_>>();
    let output = serialize_structured_results(&supplied)
        .map_err(StructuredTraceRunnerError::Serialization)?;
    Ok(StructuredTraceRun { state, output })
}

/// Runs a disposable default structured sequence with direct native CALL targets.
///
/// State selection, supplied nonces, retained journal and normal failure rows
/// follow [`run_structured_trace`]. One private native port and period sequence
/// span all prefix/target calls and are dropped with the journal on every exit.
/// No write set, rewards plan or publication capability escapes.
///
/// Before any state read, `native_factory` receives the authenticated preceding
/// concrete identity and execution period. It must bind its semantic snapshot
/// and configuration to both. Go's TraceRunner native reader factory returns
/// live sequence state even for delayed queries; the adapter must establish that
/// policy for each admitted method. A normal historical simulation port is not
/// a substitute. The initial real-port evidence covers current metadata updates
/// and `getValidator`, not all delayed methods or period-zero native sessions.
///
/// Prefix CALL/CREATE uses the existing native driver. Direct addresses selected
/// by either classifier use the native CALL driver for targets, preserving its
/// integrity/unsupported checks. These calls have no interpreter opcode rows in
/// Go. Other targets use the existing bounded traced driver; unsupported nested
/// target paths fail without returning partial JSON. Native/ordinary code and
/// consensus failures remain result rows and do not abort later targets.
/// Factory errors use [`StructuredTraceRunnerError::NativeSession`]; driver errors
/// retain the failing stage/index. OpenEthereum and RPC routing remain separate.
#[allow(clippy::too_many_arguments)]
pub fn run_structured_trace_with_native<
    R: ConcreteStateRead + ?Sized,
    B: BlockHashRead,
    A: NativeAddressClassifier,
    C: NativeAddressClassifier,
    P: NativeExecutionPort,
    F: FnOnce(
        ConcreteStateIdentity,
        rustaxa_types::FinalChainBlockNumber,
    ) -> Result<P, NativePortError>,
>(
    reader: &R,
    block_hashes: &B,
    all_native_addresses: &A,
    consensus_native_addresses: &C,
    native_factory: F,
    block: &ExecutionBlockContext,
    prefix: &[ExecutionTransaction],
    targets: &[ExecutionTransaction],
    envelope_rules: EnvelopeRules,
    profile: TaraxaProfile,
) -> Result<StructuredTraceRun, StructuredTraceRunnerError> {
    let state = reader.identity();
    let expected =
        rustaxa_types::FinalChainBlockNumber::new(block.period.as_u64().saturating_sub(1));
    if state.period != expected {
        return Err(StructuredTraceRunnerError::StatePeriodMismatch {
            state,
            expected,
            execution: block.period,
        });
    }
    let mut port =
        native_factory(state, block.period).map_err(StructuredTraceRunnerError::NativeSession)?;
    let mut sequence = PeriodConsensusSequence::new(block.period);
    let mut journal = ExecutionJournal::new(BorrowedCommitted(reader));
    for (index, transaction) in prefix.iter().enumerate() {
        let result = match transaction.kind {
            ExecutionTransactionKind::Call => execute_top_level_call_with_native(
                &mut journal,
                block_hashes,
                all_native_addresses,
                consensus_native_addresses,
                &mut port,
                &mut sequence,
                block,
                transaction,
                envelope_rules,
                profile,
            ),
            ExecutionTransactionKind::Create => execute_top_level_create_with_native(
                &mut journal,
                block_hashes,
                all_native_addresses,
                consensus_native_addresses,
                &mut port,
                &mut sequence,
                block,
                transaction,
                envelope_rules,
                profile,
            ),
            kind => Err(ExecutionDriverError::UnsupportedTransactionKind(kind)),
        };
        result.map_err(|error| StructuredTraceRunnerError::Execution {
            stage: TraceSequenceStage::Prefix,
            index,
            error,
        })?;
    }
    let mut observed = Vec::with_capacity(targets.len());
    for (index, transaction) in targets.iter().enumerate() {
        let mut collector = TraceCollector::default();
        let direct_native = transaction.receiver.is_some_and(|address| {
            all_native_addresses.is_native_address(block.period, address)
                || consensus_native_addresses.is_native_address(block.period, address)
        });
        let execution = if direct_native {
            execute_top_level_call_with_native(
                &mut journal,
                block_hashes,
                all_native_addresses,
                consensus_native_addresses,
                &mut port,
                &mut sequence,
                block,
                transaction,
                envelope_rules,
                profile,
            )
        } else {
            execute_traced(
                &mut journal,
                block_hashes,
                all_native_addresses,
                block,
                transaction,
                envelope_rules,
                profile,
                &mut collector,
            )
        }
        .map_err(|error| StructuredTraceRunnerError::Execution {
            stage: TraceSequenceStage::Target,
            index,
            error,
        })?;
        observed.push(owned_result(execution, collector));
    }
    let supplied = observed
        .iter()
        .map(|result| StructuredTraceResult {
            gas_used: result.gas_used,
            failed: result.failed,
            return_value: &result.return_value,
            events: &result.events,
        })
        .collect::<Vec<_>>();
    let output = serialize_structured_results(&supplied)
        .map_err(StructuredTraceRunnerError::Serialization)?;
    Ok(StructuredTraceRun { state, output })
}

#[allow(clippy::too_many_arguments)]
fn execute_untraced<R: ConcreteExecutionRead, B: BlockHashRead, N: NativeAddressClassifier>(
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
    native_addresses: &N,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    envelope_rules: EnvelopeRules,
    profile: TaraxaProfile,
) -> Result<TransactionExecutionResult, ExecutionDriverError> {
    match transaction.kind {
        ExecutionTransactionKind::Call => execute_top_level_call(
            journal,
            block_hashes,
            native_addresses,
            block,
            transaction,
            envelope_rules,
            profile,
        ),
        ExecutionTransactionKind::Create => execute_top_level_create(
            journal,
            block_hashes,
            native_addresses,
            block,
            transaction,
            envelope_rules,
            profile,
        ),
        kind => Err(ExecutionDriverError::UnsupportedTransactionKind(kind)),
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_traced<R: ConcreteExecutionRead, B: BlockHashRead, N: NativeAddressClassifier>(
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
    native_addresses: &N,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    envelope_rules: EnvelopeRules,
    profile: TaraxaProfile,
    observer: &mut TraceCollector,
) -> Result<TransactionExecutionResult, ExecutionDriverError> {
    match transaction.kind {
        ExecutionTransactionKind::Call => execute_top_level_call_with_trace(
            journal,
            block_hashes,
            native_addresses,
            block,
            transaction,
            envelope_rules,
            profile,
            observer,
        ),
        ExecutionTransactionKind::Create => execute_top_level_create_with_trace(
            journal,
            block_hashes,
            native_addresses,
            block,
            transaction,
            envelope_rules,
            profile,
            observer,
        ),
        kind => Err(ExecutionDriverError::UnsupportedTransactionKind(kind)),
    }
}

fn owned_result(
    execution: TransactionExecutionResult,
    collector: TraceCollector,
) -> OwnedStructuredResult {
    let (gas_used, failed, return_value) = match execution {
        TransactionExecutionResult::Executed(result) => (
            result.gas_used.as_u64(),
            !matches!(result.status, CodeExecutionStatus::Success),
            result.output,
        ),
        TransactionExecutionResult::ConsensusFailure(result) => {
            (result.gas_used.as_u64(), true, result.output)
        }
    };
    let (events, _) = collector.into_parts();
    OwnedStructuredResult {
        gas_used,
        failed,
        return_value,
        events,
    }
}
