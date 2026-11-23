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
        ExecutionDriverError, NativeAddressClassifier, NativeExecution, PeriodConsensusSequence,
        execute_admitted_call, execute_admitted_create, execute_top_level_call,
        execute_top_level_call_with_native, execute_top_level_call_with_trace,
        execute_top_level_create, execute_top_level_create_with_native,
        execute_top_level_create_with_trace,
    },
    envelope::{EnvelopeAdmission, EnvelopeRules, IntrinsicGasSchedule, admit},
    journal::ExecutionJournal,
    openethereum_trace::OpenEthereumTrace,
    profile::{CorrectionStage, TaraxaProfile},
    structured_trace::{
        StructuredTraceResult, StructuredTraceSerializationError, serialize_structured_results,
    },
    trace::{ExecutionTraceObserver, TraceCollector, TraceEvent},
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
    /// Historical wrapper has no corrected transaction-lifetime contract.
    /// Rejection occurs before reader access, admission, or native factory use.
    UnsupportedCorrectionStage(CorrectionStage),
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
    /// The reference tracer cannot produce a result without trace or vmTrace.
    InvalidOpenEthereumMode,
}

impl std::fmt::Display for StructuredTraceRunnerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidOpenEthereumMode => formatter
                .write_str("runtime error: invalid memory address or nil pointer dereference"),
            _ => write!(formatter, "structured trace runner: {self:?}"),
        }
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
    if profile.correction_stage() != CorrectionStage::Historical {
        return Err(StructuredTraceRunnerError::UnsupportedCorrectionStage(
            profile.correction_stage(),
        ));
    }
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
    run_trace_with_native(
        reader,
        block_hashes,
        all_native_addresses,
        consensus_native_addresses,
        native_factory,
        block,
        prefix,
        targets,
        envelope_rules,
        profile,
        HistoricalTraceFormat::Debug,
    )
}

/// Select the historical RPC output format for one private replay sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoricalTraceFormat {
    /// Structured trace for the Debug RPCs.
    Debug,
    /// Flat call trace, with optional VM operations.
    OpenEthereum {
        /// Include the call tree.
        trace: bool,
        /// Include VM operations and the call tree.
        vm_trace: bool,
    },
}

/// Replay prerequisites and targets on the committed parent in one private journal.
///
/// Each target has a separate collector. Replay returns an ordered JSON result
/// array and discards the journal. An unavailable checkpoint or execution
/// dependency fails the request without returning partial results.
#[allow(clippy::too_many_arguments)]
pub fn run_trace_with_native<
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
    format: HistoricalTraceFormat,
) -> Result<StructuredTraceRun, StructuredTraceRunnerError> {
    if profile.correction_stage() != CorrectionStage::Historical {
        return Err(StructuredTraceRunnerError::UnsupportedCorrectionStage(
            profile.correction_stage(),
        ));
    }
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
    let mut observed = Vec::new();
    let mut openethereum = Vec::new();
    for (index, transaction) in targets.iter().enumerate() {
        let mut collector = TraceCollector::default();
        let mut openethereum_trace = match format {
            HistoricalTraceFormat::Debug => None,
            HistoricalTraceFormat::OpenEthereum {
                trace: false,
                vm_trace: false,
            } => {
                return Err(StructuredTraceRunnerError::InvalidOpenEthereumMode);
            }
            HistoricalTraceFormat::OpenEthereum { vm_trace, .. } => {
                Some(OpenEthereumTrace::new(vm_trace))
            }
        };
        let with_frames = openethereum_trace.is_some();
        let observer: &mut dyn ExecutionTraceObserver = match openethereum_trace.as_mut() {
            Some(trace) => trace,
            None => &mut collector,
        };
        let execution = if transaction.kind == ExecutionTransactionKind::Call
            || (transaction.kind == ExecutionTransactionKind::Create && with_frames)
        {
            crate::driver::validate_profile(&journal, profile).map_err(|error| {
                StructuredTraceRunnerError::Execution {
                    stage: TraceSequenceStage::Target,
                    index,
                    error,
                }
            })?;
            match admit(
                &mut journal,
                transaction,
                envelope_rules,
                IntrinsicGasSchedule::PINNED,
            ) {
                Ok(EnvelopeAdmission::Rejected(result)) => {
                    Ok(TransactionExecutionResult::ConsensusFailure(result))
                }
                Ok(EnvelopeAdmission::Admitted(admitted)) => {
                    let execute = if transaction.kind == ExecutionTransactionKind::Call {
                        execute_admitted_call
                    } else {
                        execute_admitted_create
                    };
                    execute(
                        &mut journal,
                        block_hashes,
                        all_native_addresses,
                        block,
                        transaction,
                        &admitted,
                        profile,
                        Some(NativeExecution::new(
                            consensus_native_addresses,
                            &mut port,
                            &mut sequence,
                        )),
                        Some(observer),
                    )
                }
                Err(error) => Err(error.into()),
            }
        } else {
            execute_traced(
                &mut journal,
                block_hashes,
                all_native_addresses,
                block,
                transaction,
                envelope_rules,
                profile,
                observer,
            )
        }
        .map_err(|error| StructuredTraceRunnerError::Execution {
            stage: TraceSequenceStage::Target,
            index,
            error,
        })?;
        if let Some(trace) = openethereum_trace {
            let output = match execution {
                TransactionExecutionResult::Executed(result) => result.output,
                TransactionExecutionResult::ConsensusFailure(result) => result.output,
            };
            openethereum.push(trace.finish(&output));
        } else {
            observed.push(owned_result(execution, collector));
        }
    }
    if matches!(format, HistoricalTraceFormat::OpenEthereum { .. }) {
        return Ok(StructuredTraceRun {
            state,
            output: serde_json::to_vec(&openethereum).expect("trace contains only JSON values"),
        });
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
    observer: &mut dyn ExecutionTraceObserver,
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
