//! Disposable execution over a fixed committed historical state.
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
    profile::TaraxaProfile,
};

/// Failure to establish or execute a disposable historical simulation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SimulationError {
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
    let state = reader.identity();
    if state.period != block.period {
        return Err(SimulationError::StatePeriodMismatch {
            state,
            requested: block.period,
        });
    }
    let mut journal = ExecutionJournal::new(BorrowedCommitted(reader));
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
    let state = reader.identity();
    if state.period != block.period {
        return Err(SimulationError::StatePeriodMismatch {
            state,
            requested: block.period,
        });
    }
    let mut journal = ExecutionJournal::new(BorrowedCommitted(reader));
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
    let mut native_port = native_factory(state).map_err(SimulationError::NativeSession)?;
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
