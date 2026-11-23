//! Disposable execution over a fixed committed state.
//!
//! The public Go dry runner uses the requested block's state and replaces the
//! supplied transaction nonce with the sender's stored nonce plus one. This
//! module applies that policy before reusing the ordinary CALL/CREATE driver.
//! Each invocation owns and drops its journal, including on failure; it returns
//! no mutations, prepared state, or publication capability. Native simulation
//! additionally constructs and consumes one private native port per invocation.
//! RPC error presentation remains a separate adapter.

use rustaxa_types::concrete_state::{
    ConcreteAccountRecord, ConcreteRead, ConcreteReadError, ConcreteStateIdentity,
    ConcreteStateRead, ConcreteStorageKey,
};

use crate::{
    contracts::{
        BlockHashRead, ExecutionBlockContext, ExecutionTransaction, ExecutionTransactionKind,
        NativeExecutionPort, NativePortError, TransactionExecutionResult,
    },
    driver::{
        ExecutionDriverError, NativeAddressClassifier, PeriodConsensusSequence,
        execute_top_level_call, execute_top_level_call_with_native, execute_top_level_create,
        execute_top_level_create_with_native,
    },
    envelope::EnvelopeRules,
    journal::{ExecutionJournal, JournalError},
    profile::{CorrectionStage, TaraxaProfile},
};

/// Failure to establish or execute a disposable simulation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SimulationError {
    /// Historical wrapper has no corrected transaction-lifetime contract.
    /// Rejection occurs before reader access, admission, or native factory use.
    UnsupportedCorrectionStage(CorrectionStage),
    /// A corrected constructor requires an explicit non-Historical profile.
    HistoricalCorrectionStage,
    /// The committed reader no longer matches the selected period and root.
    StateIdentityMismatch {
        /// Immutable identity selected by the caller.
        expected: ConcreteStateIdentity,
        /// Identity reported by the supplied reader.
        actual: ConcreteStateIdentity,
    },
    /// Native origin facts differ from the selected simulation facts.
    NativeBindingMismatch,
    /// The native session does not have the selected correction identity.
    NativeCorrectionStageMismatch {
        /// Immutable simulation stage.
        expected: CorrectionStage,
        /// Construction-bound native stage, if supplied.
        actual: Option<CorrectionStage>,
    },
    /// The block context does not select the supplied committed reader's period.
    /// No account read or execution has occurred.
    StatePeriodMismatch {
        /// Actual fixed reader identity, including its authenticated root.
        state: ConcreteStateIdentity,
        /// Requested execution block number.
        requested: rustaxa_types::FinalChainBlockNumber,
    },
    /// Loading the authoritative sender failed; absence is handled normally.
    Sender(JournalError),
    /// Creating the private native session failed before execution began.
    NativeSession(NativePortError),
    /// Infrastructure, unsupported dispatch, or driver invariants failed.
    Execution(ExecutionDriverError),
}

/// Result bound to the committed state actually used by the simulation.
///
/// Consensus and code failures remain ordinary execution outcomes. Return data,
/// logs and attempted creation addresses describe the discarded execution only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulationResult {
    /// Fixed committed period/root; simulations never advance this identity.
    pub state: ConcreteStateIdentity,
    /// Exact driver result, before RPC-specific error-string conversion.
    pub execution: TransactionExecutionResult,
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

/// Simulates ordinary CALL or CREATE without exposing any state-write result.
///
/// `reader` must implement the committed-state contract, not only the prepared
/// execution-view port. `block` must identify that same period; its other facts,
/// the historical envelope/profile rules, native registry and block-hash reader
/// are supplied by the application. Registered native targets fail explicitly
/// through the ordinary driver until the native simulation adapter is supplied.
///
/// The caller's transaction is unchanged. Its nonce is ignored and replaced in
/// a private copy by the full-width stored sender nonce plus one, matching Go
/// `state_dry_runner.DryRunner.Apply`. Gas price, value and gas cap are preserved,
/// so affordability and intrinsic-gas errors retain normal envelope behavior.
/// System inputs are rejected. A new journal is created for every call and
/// always discarded, making repeated gas probes independent. This API does not
/// perform RPC defaults, format revert reasons, estimate gas or produce traces.
#[allow(clippy::too_many_arguments)]
pub fn simulate_ordinary<
    R: ConcreteStateRead + ?Sized,
    B: BlockHashRead,
    N: NativeAddressClassifier,
>(
    reader: &R,
    block_hashes: &B,
    native_addresses: &N,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    envelope_rules: EnvelopeRules,
    profile: TaraxaProfile,
) -> Result<SimulationResult, SimulationError> {
    if profile.correction_stage() != CorrectionStage::Historical {
        return Err(SimulationError::UnsupportedCorrectionStage(
            profile.correction_stage(),
        ));
    }
    simulate_ordinary_inner(
        reader,
        block_hashes,
        native_addresses,
        block,
        transaction,
        envelope_rules,
        profile,
    )
}

#[allow(clippy::too_many_arguments)]
fn simulate_ordinary_inner<
    R: ConcreteStateRead + ?Sized,
    B: BlockHashRead,
    N: NativeAddressClassifier,
>(
    reader: &R,
    block_hashes: &B,
    native_addresses: &N,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    envelope_rules: EnvelopeRules,
    profile: TaraxaProfile,
) -> Result<SimulationResult, SimulationError> {
    let state = reader.identity();
    if state.period != block.period {
        return Err(SimulationError::StatePeriodMismatch {
            state,
            requested: block.period,
        });
    }
    let mut journal = ExecutionJournal::new_with_correction_stage(
        BorrowedCommitted(reader),
        profile.correction_stage(),
    );
    let mut request = transaction.clone();
    request.nonce = journal
        .account(request.sender)
        .map_err(SimulationError::Sender)?
        .nonce
        .next();
    let execution = match request.kind {
        ExecutionTransactionKind::Call => execute_top_level_call(
            &mut journal,
            block_hashes,
            native_addresses,
            block,
            &request,
            envelope_rules,
            profile,
        ),
        ExecutionTransactionKind::Create => execute_top_level_create(
            &mut journal,
            block_hashes,
            native_addresses,
            block,
            &request,
            envelope_rules,
            profile,
        ),
        kind => Err(ExecutionDriverError::UnsupportedTransactionKind(kind)),
    }
    .map_err(SimulationError::Execution)?;
    Ok(SimulationResult { state, execution })
}

/// Simulates CALL or CREATE with one disposable native session and journal.
///
/// The historical state, nonce and envelope policies are identical to
/// [`simulate_ordinary`]. After period validation and the sender read,
/// `native_factory` receives the exact committed identity and must construct a
/// fresh, unpublished port bound to that historical state. The adapter owns
/// authentication of its semantic native snapshot and any delayed read views;
/// it must not write committed state or reuse a mutable session from another
/// probe. A pending-next-block session is not a historical simulation session.
///
/// This function consumes the resulting port alongside a fresh zero-based
/// native sequence and journal, including on error. None can be recovered from
/// the result. The factory is not called for a period mismatch, failed sender
/// read or unsupported system input. Native infrastructure failures retain the
/// driver's error rather than becoming a code execution result. This is an
/// explicit composition API; it does not select application or production routes.
#[allow(clippy::too_many_arguments)]
pub fn simulate_with_native<
    R: ConcreteStateRead + ?Sized,
    B: BlockHashRead,
    A: NativeAddressClassifier,
    C: NativeAddressClassifier,
    P: NativeExecutionPort,
    F: FnOnce(ConcreteStateIdentity) -> Result<P, NativePortError>,
>(
    reader: &R,
    block_hashes: &B,
    all_native_addresses: &A,
    consensus_native_addresses: &C,
    native_factory: F,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    envelope_rules: EnvelopeRules,
    profile: TaraxaProfile,
) -> Result<SimulationResult, SimulationError> {
    if profile.correction_stage() != CorrectionStage::Historical {
        return Err(SimulationError::UnsupportedCorrectionStage(
            profile.correction_stage(),
        ));
    }
    simulate_with_native_inner(
        reader,
        block_hashes,
        all_native_addresses,
        consensus_native_addresses,
        |state| native_factory(state).map_err(SimulationError::NativeSession),
        block,
        transaction,
        envelope_rules,
        profile,
    )
}

#[allow(clippy::too_many_arguments)]
fn simulate_with_native_inner<
    R: ConcreteStateRead + ?Sized,
    B: BlockHashRead,
    A: NativeAddressClassifier,
    C: NativeAddressClassifier,
    P: NativeExecutionPort,
    F: FnOnce(ConcreteStateIdentity) -> Result<P, SimulationError>,
>(
    reader: &R,
    block_hashes: &B,
    all_native_addresses: &A,
    consensus_native_addresses: &C,
    native_factory: F,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    envelope_rules: EnvelopeRules,
    profile: TaraxaProfile,
) -> Result<SimulationResult, SimulationError> {
    let state = reader.identity();
    if state.period != block.period {
        return Err(SimulationError::StatePeriodMismatch {
            state,
            requested: block.period,
        });
    }
    let mut journal = ExecutionJournal::new_with_correction_stage(
        BorrowedCommitted(reader),
        profile.correction_stage(),
    );
    let mut request = transaction.clone();
    request.nonce = journal
        .account(request.sender)
        .map_err(SimulationError::Sender)?
        .nonce
        .next();
    if !matches!(
        request.kind,
        ExecutionTransactionKind::Call | ExecutionTransactionKind::Create
    ) {
        return Err(SimulationError::Execution(
            ExecutionDriverError::UnsupportedTransactionKind(request.kind),
        ));
    }
    let mut native_port = native_factory(state)?;
    let mut sequence = PeriodConsensusSequence::new(state.period);
    let execution = match request.kind {
        ExecutionTransactionKind::Call => execute_top_level_call_with_native(
            &mut journal,
            block_hashes,
            all_native_addresses,
            consensus_native_addresses,
            &mut native_port,
            &mut sequence,
            block,
            &request,
            envelope_rules,
            profile,
        ),
        ExecutionTransactionKind::Create => execute_top_level_create_with_native(
            &mut journal,
            block_hashes,
            all_native_addresses,
            consensus_native_addresses,
            &mut native_port,
            &mut sequence,
            block,
            &request,
            envelope_rules,
            profile,
        ),
        kind => Err(ExecutionDriverError::UnsupportedTransactionKind(kind)),
    }
    .map_err(SimulationError::Execution)?;
    Ok(SimulationResult { state, execution })
}

/// Immutable selection for one corrected simulation and all its gas probes.
///
/// Equality checks these facts. It does not authenticate a native semantic
/// snapshot. The application adapter must verify the finalized root, chain
/// configuration, profile, envelope rules and delayed views independently.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulationBinding {
    /// Exact committed period and root selected by the caller.
    pub state: ConcreteStateIdentity,
    /// Fixed block facts, including the chain identifier.
    pub block: ExecutionBlockContext,
    /// Fixed instruction phase and non-Historical correction stage.
    pub profile: TaraxaProfile,
    /// Fixed envelope admission rules.
    pub envelope_rules: EnvelopeRules,
}

/// Native port with independently verified disposable simulation provenance.
///
/// Implementations must own a fresh, unpublished simulation session, selected
/// from the finalized state in the binding. A pending-period session is not
/// sufficient. The owner must authenticate the semantic snapshot and delayed
/// reads before returning this port. Copying the requested binding alone does
/// not meet this contract. This module checks structural equality and the
/// native port's construction-bound stage; it cannot inspect semantic origins.
pub trait CorrectedSimulationNative: NativeExecutionPort {
    /// Returns the independently verified origin facts for this private port.
    fn simulation_binding(&self) -> &SimulationBinding;
}

/// Reusable immutable selection for disposable corrected CALL and CREATE.
///
/// Each invocation creates and drops a fresh journal, native sequence and port.
/// The committed reader must remain immutable for this object's lifetime.
/// Results contain execution facts only and have no publication capability.
/// This API does not select an application, bridge or RPC route.
pub struct CorrectedSimulation<'a, R: ConcreteStateRead + ?Sized> {
    reader: &'a R,
    binding: SimulationBinding,
}

impl<'a, R: ConcreteStateRead + ?Sized> CorrectedSimulation<'a, R> {
    /// Selects one explicit corrected profile and exact committed state.
    ///
    /// Historical profiles fail before reader access. The expected identity
    /// must match the reader's period and root, and the block must select that
    /// period. No account read or native factory runs during construction.
    pub fn new(reader: &'a R, binding: SimulationBinding) -> Result<Self, SimulationError> {
        if binding.profile.correction_stage() == CorrectionStage::Historical {
            return Err(SimulationError::HistoricalCorrectionStage);
        }
        let actual = reader.identity();
        if actual != binding.state {
            return Err(SimulationError::StateIdentityMismatch {
                expected: binding.state,
                actual,
            });
        }
        if binding.state.period != binding.block.period {
            return Err(SimulationError::StatePeriodMismatch {
                state: binding.state,
                requested: binding.block.period,
            });
        }
        Ok(Self { reader, binding })
    }

    /// Returns the fixed selection without any state-write capability.
    pub fn binding(&self) -> &SimulationBinding {
        &self.binding
    }

    fn validate_reader(&self) -> Result<(), SimulationError> {
        let actual = self.reader.identity();
        if actual != self.binding.state {
            return Err(SimulationError::StateIdentityMismatch {
                expected: self.binding.state,
                actual,
            });
        }
        Ok(())
    }

    /// Executes ordinary CALL or CREATE with a new corrected journal.
    ///
    /// Sender nonce substitution, value and gas admission, outputs and code
    /// failures use the same driver as the Historical wrappers.
    pub fn ordinary<B: BlockHashRead, N: NativeAddressClassifier>(
        &self,
        block_hashes: &B,
        native_addresses: &N,
        transaction: &ExecutionTransaction,
    ) -> Result<SimulationResult, SimulationError> {
        self.validate_reader()?;
        simulate_ordinary_inner(
            self.reader,
            block_hashes,
            native_addresses,
            &self.binding.block,
            transaction,
            self.binding.envelope_rules,
            self.binding.profile,
        )
    }

    /// Executes CALL or CREATE with one fresh native simulation port.
    ///
    /// The factory owns origin authentication described by
    /// [`CorrectedSimulationNative`]. Structural binding and exact native stage
    /// checks occur before driver admission, prepare or invoke. A sender read
    /// and unsupported-kind guard precede factory use. All private execution
    /// state is dropped on success, ordinary failure and infrastructure error.
    pub fn with_native<B, A, C, P, F>(
        &self,
        block_hashes: &B,
        all_native_addresses: &A,
        consensus_native_addresses: &C,
        native_factory: F,
        transaction: &ExecutionTransaction,
    ) -> Result<SimulationResult, SimulationError>
    where
        B: BlockHashRead,
        A: NativeAddressClassifier,
        C: NativeAddressClassifier,
        P: CorrectedSimulationNative,
        F: FnOnce(&SimulationBinding) -> Result<P, NativePortError>,
    {
        self.validate_reader()?;
        simulate_with_native_inner(
            self.reader,
            block_hashes,
            all_native_addresses,
            consensus_native_addresses,
            |_| {
                let port = native_factory(&self.binding).map_err(SimulationError::NativeSession)?;
                let expected = self.binding.profile.correction_stage();
                if port.simulation_binding() != &self.binding {
                    return Err(SimulationError::NativeBindingMismatch);
                }
                if port.correction_stage() != Some(expected) {
                    return Err(SimulationError::NativeCorrectionStageMismatch {
                        expected,
                        actual: port.correction_stage(),
                    });
                }
                Ok(port)
            },
            &self.binding.block,
            transaction,
            self.binding.envelope_rules,
            self.binding.profile,
        )
    }

    /// Estimates gas with fresh execution state for each candidate.
    ///
    /// Only completed execution outcomes enter the caller's presentation mapper.
    /// Native, reader and other infrastructure errors bypass that mapper and
    /// return [`crate::estimate::EstimateError::Callback`] immediately. The
    /// mapper owns normal consensus/code error text and RPC presentation policy.
    /// The factory must create a new private port for every probe, including
    /// after an earlier failed candidate; mutable sessions cannot be reused.
    pub fn estimate_with_native<B, A, C, P, F, M>(
        &self,
        block_hashes: &B,
        all_native_addresses: &A,
        consensus_native_addresses: &C,
        mut native_factory: F,
        transaction: &ExecutionTransaction,
        mut map_result: M,
    ) -> Result<u64, crate::estimate::EstimateError<SimulationError>>
    where
        B: BlockHashRead,
        A: NativeAddressClassifier,
        C: NativeAddressClassifier,
        P: CorrectedSimulationNative,
        F: FnMut(&SimulationBinding) -> Result<P, NativePortError>,
        M: FnMut(TransactionExecutionResult) -> crate::estimate::EstimateProbe,
    {
        crate::estimate::estimate_gas(transaction.gas_limit.as_u64(), |gas| {
            let mut request = transaction.clone();
            request.gas_limit = gas.into();
            let result = self.with_native(
                block_hashes,
                all_native_addresses,
                consensus_native_addresses,
                &mut native_factory,
                &request,
            )?;
            Ok(map_result(result.execution))
        })
    }
}
