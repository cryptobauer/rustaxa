//! Apply a staged native result through the existing execution journal lanes.
//!
//! The frame opens the checkpoint, moves call value and settles the returned
//! status. The native port owns ordered invocation identities and staged kernel
//! state. This adapter owns neither operation and cannot register a precompile,
//! choose consensus inputs or publish state. Any integrity failure aborts the
//! pending period; a partially advanced journal/session must not be retried.

use rustaxa_types::{
    FinalChainGas,
    concrete_state::{ConcreteRead, ConcreteStorageKey, execution::ConcreteExecutionRead},
};

use crate::{
    contracts::{
        CodeExecutionError, CodeExecutionStatus, NativeExecutionPort, NativeInvocation,
        NativeInvocationResult, NativePortError, NativeResultValidationError, NativeStatus,
    },
    journal::{ExecutionJournal, JournalError},
};

/// Native action completion for the caller-owned frame checkpoint.
///
/// On success the frame commits its checkpoint. On normal failure it rolls back
/// ordinary account/log effects while preserving the reference's raw lane. Both
/// native failure and insufficient gas return the remaining child gas here;
/// generic exceptional-bytecode insertion must not silently burn it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeFrameOutcome {
    /// Success, exact native contract failure or native gas-admission failure.
    pub status: CodeExecutionStatus,
    /// Accepted native quote, including unfunded calls, retained for the existing
    /// concrete invocation transcript. It is not necessarily charged gas.
    pub required_gas: FinalChainGas,
    /// Unused supplied action gas; no transaction/base CALL cost is included.
    pub gas_left: FinalChainGas,
    /// Exact return bytes, including bytes returned with a contract failure.
    pub output: Vec<u8>,
}

/// Integrity/infrastructure failure requiring whole-period discard.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeAdapterError {
    /// The staged port could not prepare or execute safely.
    Port(NativePortError),
    /// A quote/result did not satisfy the accepted invocation and gas contract.
    InvalidResult(NativeResultValidationError),
    /// Authoritative journal read or ordered ordinary mutation failed.
    Journal(JournalError),
    /// A raw mutation was not based on the currently visible raw lane.
    RawExpectation {
        address: [u8; 20],
        key: ConcreteStorageKey,
        expected: ConcreteRead<Vec<u8>>,
        observed: ConcreteRead<Vec<u8>>,
    },
}

impl std::fmt::Display for NativeAdapterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "native journal adapter: {self:?}")
    }
}
impl std::error::Error for NativeAdapterError {}

/// Prepares/invokes one native operation and applies its validated effects.
///
/// The caller must already own the native call's frame checkpoint and apply the
/// returned success/failure to it. This function never transfers call value,
/// changes envelope nonces, registers addresses or advances invocation sequence.
/// It validates quote identity before invoking and result gas before effects.
/// Ordinary mutations apply in their supplied order, followed by exact-expected
/// raw mutations and logs, including normal contract-failure effects. A mismatch
/// aborts the pending period; earlier port/journal changes cannot be reused.
pub fn invoke_native<R: ConcreteExecutionRead, N: NativeExecutionPort + ?Sized>(
    journal: &mut ExecutionJournal<R>,
    port: &mut N,
    invocation: &NativeInvocation,
) -> Result<NativeFrameOutcome, NativeAdapterError> {
    let quote = port
        .prepare(invocation, journal)
        .map_err(NativeAdapterError::Port)?;
    if quote.invocation != invocation.id {
        return Err(NativeAdapterError::InvalidResult(
            NativeResultValidationError::InvocationMismatch,
        ));
    }
    let result = port
        .invoke(invocation, quote, journal)
        .map_err(NativeAdapterError::Port)?;
    result
        .validate(invocation, quote)
        .map_err(NativeAdapterError::InvalidResult)?;
    let NativeInvocationResult::Completed(outcome) = result else {
        return Ok(NativeFrameOutcome {
            status: CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas),
            required_gas: quote.required_gas,
            gas_left: invocation.supplied_gas,
            output: Vec::new(),
        });
    };
    journal
        .apply_native_account_mutations(&outcome.account_mutations)
        .map_err(NativeAdapterError::Journal)?;
    for mutation in outcome.raw_mutations {
        let observed = journal
            .raw_storage(mutation.address, mutation.key)
            .map_err(NativeAdapterError::Journal)?;
        if observed != mutation.expected {
            return Err(NativeAdapterError::RawExpectation {
                address: mutation.address,
                key: mutation.key,
                expected: mutation.expected,
                observed,
            });
        }
        journal
            .set_raw_storage(mutation.address, mutation.key, mutation.operation)
            .map_err(NativeAdapterError::Journal)?;
    }
    for log in outcome.logs {
        journal.push_log(log);
    }
    Ok(NativeFrameOutcome {
        required_gas: quote.required_gas,
        status: match outcome.status {
            NativeStatus::Success => CodeExecutionStatus::Success,
            NativeStatus::ContractFailure(error) => {
                CodeExecutionStatus::Failure(CodeExecutionError::Native(error))
            }
        },
        // Result validation proves supplied >= quote and charged == quote.
        gas_left: (invocation.supplied_gas.as_u64() - quote.required_gas.as_u64()).into(),
        output: outcome.output,
    })
}
