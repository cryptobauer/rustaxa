//! Disposable execution over a fixed committed historical state.
//!
//! The public Go dry runner uses the requested block's state and replaces the
//! supplied transaction nonce with the sender's stored nonce plus one. This
//! module applies that policy before reusing the ordinary CALL/CREATE driver.
//! Each invocation owns and drops its journal, including on failure; it returns
//! no mutations, prepared state, or publication capability. Native dispatch and
//! RPC error presentation are separate adapters, not implicit legacy fallbacks.

use rustaxa_types::concrete_state::{
    ConcreteAccountRecord, ConcreteRead, ConcreteReadError, ConcreteStateIdentity,
    ConcreteStateRead, ConcreteStorageKey,
};

use crate::{
    contracts::{
        BlockHashRead, ExecutionBlockContext, ExecutionTransaction, ExecutionTransactionKind,
        TransactionExecutionResult,
    },
    driver::{
        ExecutionDriverError, NativeAddressClassifier, execute_top_level_call,
        execute_top_level_create,
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
