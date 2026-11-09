//! Apply a staged native result through the existing execution journal lanes.
//!
//! The frame opens the checkpoint, moves call value and settles the returned
//! status. The driver allocates ordered invocation identities; the native port
//! validates them and owns staged kernel state. This adapter owns neither
//! operation and cannot register a precompile,
//! choose consensus inputs or publish state. Any integrity failure aborts the
//! pending period; a partially advanced journal/session must not be retried.

use rustaxa_types::{
    FinalChainGas,
    concrete_state::{ConcreteRead, ConcreteStorageKey, execution::ConcreteExecutionRead},
};

use crate::{
    contracts::{
        CodeExecutionError, CodeExecutionStatus, ConsensusNativeDisposition,
        ConsensusNativeObservation, NativeExecutionPort, NativeInputMutation, NativeInvocation,
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
    /// Ordered mutations of the borrowed input slice, before caller return copy.
    pub input_mutations: Vec<NativeInputMutation>,
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
    /// Borrowed input metadata did not match the completed invocation.
    InputMutation { offset: usize },
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
    let _duration_scope = crate::journal::duration_journal_native_scope(invocation);
    journal.begin_native_invocation(invocation.id);
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
        crate::journal::duration_native_insufficient_gas();
        journal.record_native_invocation(ConsensusNativeObservation {
            invocation: invocation.clone(),
            required_gas: quote.required_gas,
            gas_used: 0_u64.into(),
            status: CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas),
            output: Vec::new(),
            logs: Vec::new(),
            disposition: ConsensusNativeDisposition::OwnFrameReverted,
        });
        return Ok(NativeFrameOutcome {
            input_mutations: Vec::new(),
            status: CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas),
            required_gas: quote.required_gas,
            gas_left: invocation.supplied_gas,
            output: Vec::new(),
        });
    };
    crate::journal::duration_native_outcome_boundary(true);
    crate::journal::duration_native_outcome_status(invocation, &outcome.status);
    for mutation in &outcome.account_mutations {
        crate::journal::duration_native_account_event("outcome_account", mutation);
    }
    for mutation in &outcome.raw_mutations {
        crate::journal::duration_native_raw_event("outcome_raw", mutation);
    }
    crate::journal::duration_native_outcome_boundary(false);
    let input_mutations = port
        .input_mutations(invocation.id)
        .map_err(NativeAdapterError::Port)?;
    validate_input_mutations(&invocation.input, &input_mutations)?;
    for (ordinal, mutation) in outcome.account_mutations.iter().enumerate() {
        crate::journal::duration_native_apply_account_event(
            true,
            invocation,
            ordinal as u64,
            mutation,
        );
        journal
            .apply_native_account_mutations(std::slice::from_ref(mutation))
            .map_err(NativeAdapterError::Journal)?;
        crate::journal::duration_native_apply_account_event(
            false,
            invocation,
            ordinal as u64,
            mutation,
        );
    }
    for (ordinal, mutation) in outcome.raw_mutations.into_iter().enumerate() {
        crate::journal::duration_native_apply_raw_event(
            true,
            invocation,
            ordinal as u64,
            &mutation,
        );
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
        let evidence = crate::journal::duration_journal_is_armed().then(|| mutation.clone());
        journal
            .set_raw_storage(mutation.address, mutation.key, mutation.operation)
            .map_err(NativeAdapterError::Journal)?;
        if let Some(evidence) = &evidence {
            crate::journal::duration_native_raw_event("applied_raw", evidence);
            crate::journal::duration_native_apply_raw_event(
                false,
                invocation,
                ordinal as u64,
                evidence,
            );
        }
    }
    let status = match outcome.status {
        NativeStatus::Success => CodeExecutionStatus::Success,
        NativeStatus::ContractFailure(error) => {
            CodeExecutionStatus::Failure(CodeExecutionError::Native(error))
        }
    };
    journal.record_native_invocation(ConsensusNativeObservation {
        invocation: invocation.clone(),
        required_gas: quote.required_gas,
        gas_used: outcome.gas_used,
        disposition: if status == CodeExecutionStatus::Success {
            ConsensusNativeDisposition::Normal
        } else {
            ConsensusNativeDisposition::OwnFrameReverted
        },
        status: status.clone(),
        output: outcome.output.clone(),
        logs: outcome.logs.clone(),
    });
    for log in outcome.logs {
        journal.push_log(log);
    }
    Ok(NativeFrameOutcome {
        input_mutations,
        required_gas: quote.required_gas,
        status,
        // Result validation proves supplied >= quote and charged == quote.
        gas_left: (invocation.supplied_gas.as_u64() - quote.required_gas.as_u64()).into(),
        output: outcome.output,
    })
}

fn validate_input_mutations(
    input: &[u8],
    mutations: &[NativeInputMutation],
) -> Result<(), NativeAdapterError> {
    let mut working = input.to_vec();
    for mutation in mutations {
        let byte = working
            .get_mut(mutation.offset)
            .ok_or(NativeAdapterError::InputMutation {
                offset: mutation.offset,
            })?;
        if *byte != mutation.expected {
            return Err(NativeAdapterError::InputMutation {
                offset: mutation.offset,
            });
        }
        *byte = mutation.replacement;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_integration_borrowed_input_validation_is_atomic_and_ordered() {
        let canonical = vec![27, 8];
        let original = canonical.clone();
        let change = NativeInputMutation {
            offset: 0,
            expected: 27,
            replacement: 0,
        };
        assert_eq!(validate_input_mutations(&canonical, &[change]), Ok(()));
        assert_eq!(
            validate_input_mutations(
                &canonical,
                &[
                    change,
                    NativeInputMutation {
                        offset: 0,
                        expected: 0,
                        replacement: 229
                    }
                ]
            ),
            Ok(())
        );
        for invalid in [
            NativeInputMutation {
                offset: 2,
                expected: 8,
                replacement: 0,
            },
            NativeInputMutation {
                offset: 1,
                expected: 9,
                replacement: 0,
            },
        ] {
            assert_eq!(
                validate_input_mutations(&canonical, &[change, invalid]),
                Err(NativeAdapterError::InputMutation {
                    offset: invalid.offset
                })
            );
            assert_eq!(canonical, original);
        }
    }
}
