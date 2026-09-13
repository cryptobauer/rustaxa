//! Bounded top-level Taraxa bytecode execution through REVM.
//!
//! This driver composes the reviewed envelope, profile and journal for an
//! ordinary top-level CALL or CREATE. It executes real REVM legacy bytecode,
//! drives nested ordinary CALL/CREATE frames iteratively, and settles interpreter
//! gas/refunds once. The default entry points keep native dispatch unavailable;
//! explicit opt-in entry points accept a consensus-native port and period-local
//! sequence. They also route stateless addresses 1–9 and profile-selected BLS,
//! P-256 and Falcon without using that port or sequence. SELFDESTRUCT uses the
//! journal-owned historical lifecycle.
//!
//! CALL-family opcode preparation reads authoritative account metadata while
//! deferring referenced code bytes. The driver applies Taraxa's depth and
//! balance pre-entry checks first, then loads and validates code before any
//! child interpreter starts. EXTCODE operations continue to use the host's
//! immediate authoritative code path.

use num_bigint::BigInt;
use revm::{
    bytecode::Bytecode,
    handler::handle_reservoir_remaining_gas,
    interpreter::{
        CallInput, CallInputs, CallScheme as RevmCallScheme, CreateInputs, FrameInput, Gas,
        InputsImpl, InstructionResult, Interpreter, InterpreterAction, InterpreterResult,
        SharedMemory,
        interpreter::{EthInterpreter, ExtBytecode},
        interpreter_types::{Jumps, LoopControl, ReturnData},
    },
    primitives::{Address, Bytes, U256, hardfork::SpecId},
};
use rustaxa_types::{FinalChainGas, concrete_state::execution::ConcreteExecutionRead};

use crate::{
    bls::{BlsPrecompile, BlsRegistry, PreparedBlsCall},
    contracts::{
        BlockHashRead, CodeExecutionError, CodeExecutionStatus, ExecutionBlockContext,
        ExecutionTransaction, ExecutionTransactionKind, ExecutionValue, NativeCallKind,
        NativeExecutionPort, NativeInvocation, NativeInvocationId, NativeInvocationResult,
        NativePortError, NativeResultValidationError, NativeStatus, StatelessInvocation,
        StatelessInvocationId, TransactionExecutionResult,
    },
    curve_precompiles::{OriginalCurvePrecompile, PreparedCurvePrecompileCall},
    envelope::{
        AdmittedTransaction, EnvelopeAdmission, EnvelopeError, EnvelopeRules, FrameSettlement,
        FrameSettlementStatus, IntrinsicGasSchedule, admit, settle,
    },
    falcon::{FALCON_VERIFY_ADDRESS, PreparedFalconCall},
    frame::{CreateScheme, create_address, settle_code_deposit},
    host::{HostError, JournalHost},
    journal::{ExecutionJournal, JournalCheckpoint, JournalError},
    modexp::PreparedModexpCall,
    native::{NativeAdapterError, NativeFrameOutcome, invoke_native},
    p256::{P256_VERIFY_ADDRESS, PreparedP256Call},
    profile::TaraxaProfile,
    stateless::{OriginalStatelessPrecompile, PreparedStatelessCall},
};

/// Kind of interpreter frame request not yet implemented by this bounded driver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingFrameKind {
    /// A CALL-family action could not be routed by the configured native boundary.
    Call,
    /// A CREATE-family action could not be represented by the iterative driver.
    Create,
    /// An empty frame request is not valid for this execution path.
    Empty,
}

/// Failure outside normal consensus or bytecode completion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionDriverError {
    /// Envelope admission or settlement could not mutate the journal safely.
    Envelope(EnvelopeError),
    /// A direct frame mutation/checkpoint operation failed.
    Journal(JournalError),
    /// The host could not load or mutate authoritative state.
    Host(HostError),
    /// This slice executes ordinary top-level calls only.
    UnsupportedTransactionKind(ExecutionTransactionKind),
    /// A native/precompile target requires the later typed native dispatcher.
    NativeCallUnavailable { address: [u8; 20] },
    /// A consensus-native classifier selected an address outside the complete native set.
    NativeClassifierMismatch { address: [u8; 20] },
    /// A reviewed stateless address was also selected by the consensus-native classifier.
    NativeClassifierOverlap { address: [u8; 20] },
    /// The period-local native sequence was used with another pending period.
    NativeSequencePeriod {
        /// Period bound to the sequence owner.
        expected: rustaxa_types::FinalChainBlockNumber,
        /// Period supplied by the execution block.
        observed: rustaxa_types::FinalChainBlockNumber,
    },
    /// The period-local consensus-native sequence cannot advance further.
    NativeSequenceOverflow,
    /// A transaction-local stateless invocation ordinal cannot advance further.
    StatelessOrdinalOverflow,
    /// Native preparation, execution or effect application failed.
    Native(NativeAdapterError),
    /// A stateless helper could not prepare or execute safely.
    Stateless(NativePortError),
    /// A stateless helper returned a result inconsistent with its exact quote.
    StatelessResult(NativeResultValidationError),
    /// A stateless helper returned state or log effects outside its pure contract.
    StatelessEffects { address: [u8; 20] },
    /// REVM yielded a malformed or explicitly unsupported frame action.
    PendingFrameUnavailable(PendingFrameKind),
    /// REVM returned a terminal category that has no reviewed Taraxa mapping yet.
    UnsupportedTerminal(String),
    /// The pinned reference would panic for this opcode; discard the session.
    ReferenceInstructionPanic(u8),
    /// The final successful root frame retained a negative refund aggregate.
    NegativeRootRefund(i64),
}

/// Application/profile-owned classification of native/precompile addresses.
pub trait NativeAddressClassifier {
    /// Returns whether `address` is native at the supplied finalized period.
    fn is_native_address(
        &self,
        period: rustaxa_types::FinalChainBlockNumber,
        address: [u8; 20],
    ) -> bool;
}

struct NativeExecution<'a> {
    consensus_addresses: &'a dyn NativeAddressClassifier,
    port: &'a mut dyn NativeExecutionPort,
    sequence: &'a mut PeriodConsensusSequence,
    stateless_sequence: TransactionStatelessSequence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeRoute {
    Ordinary,
    Consensus,
    Stateless,
    Unsupported,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct TransactionStatelessSequence {
    next_ordinal: u64,
}

impl TransactionStatelessSequence {
    fn allocate(
        &mut self,
        transaction: rustaxa_types::FinalChainTransactionPosition,
    ) -> Result<StatelessInvocationId, ExecutionDriverError> {
        let ordinal = self.next_ordinal;
        self.next_ordinal = ordinal
            .checked_add(1)
            .ok_or(ExecutionDriverError::StatelessOrdinalOverflow)?;
        Ok(StatelessInvocationId {
            transaction,
            ordinal,
        })
    }
}

fn bls_registry(profile: TaraxaProfile) -> Option<BlsRegistry> {
    if profile.cacti() {
        Some(BlsRegistry::Cacti)
    } else if profile.ficus() {
        Some(BlsRegistry::Ficus)
    } else {
        None
    }
}

fn is_reviewed_stateless_address(address: [u8; 20], profile: TaraxaProfile) -> bool {
    OriginalStatelessPrecompile::at_address(address).is_some()
        || (address[..19] == [0; 19] && address[19] == 5)
        || OriginalCurvePrecompile::at_address(address).is_some()
        || (profile.cacti() && matches!(address, P256_VERIFY_ADDRESS | FALCON_VERIFY_ADDRESS))
        || bls_registry(profile)
            .is_some_and(|registry| BlsPrecompile::at_address(registry, address).is_some())
}

fn validate_native_period(
    sequence: &PeriodConsensusSequence,
    period: rustaxa_types::FinalChainBlockNumber,
) -> Result<(), ExecutionDriverError> {
    if sequence.period != period {
        return Err(ExecutionDriverError::NativeSequencePeriod {
            expected: sequence.period,
            observed: period,
        });
    }
    Ok(())
}

fn classify_native<N: NativeAddressClassifier>(
    all_native_addresses: &N,
    native: Option<&NativeExecution<'_>>,
    period: rustaxa_types::FinalChainBlockNumber,
    address: [u8; 20],
    profile: TaraxaProfile,
) -> Result<NativeRoute, ExecutionDriverError> {
    let all_native = all_native_addresses.is_native_address(period, address);
    let consensus_native = native.is_some_and(|execution| {
        execution
            .consensus_addresses
            .is_native_address(period, address)
    });
    if consensus_native && !all_native {
        return Err(ExecutionDriverError::NativeClassifierMismatch { address });
    }
    let stateless =
        native.is_some() && all_native && is_reviewed_stateless_address(address, profile);
    if consensus_native && stateless {
        return Err(ExecutionDriverError::NativeClassifierOverlap { address });
    }
    Ok(if consensus_native {
        NativeRoute::Consensus
    } else if stateless {
        NativeRoute::Stateless
    } else if all_native {
        NativeRoute::Unsupported
    } else {
        NativeRoute::Ordinary
    })
}

fn native_frame_status(outcome: &NativeFrameOutcome) -> FrameSettlementStatus {
    match &outcome.status {
        CodeExecutionStatus::Success => FrameSettlementStatus::Success,
        CodeExecutionStatus::Failure(error) => FrameSettlementStatus::CodeFailure(error.clone()),
    }
}

fn invoke_stateless(
    invocation: StatelessInvocation,
    profile: TaraxaProfile,
) -> Result<NativeFrameOutcome, ExecutionDriverError> {
    let expected = invocation.clone();
    let (quote, result) = if OriginalStatelessPrecompile::at_address(invocation.contract).is_some()
    {
        let prepared =
            PreparedStatelessCall::prepare(invocation).map_err(ExecutionDriverError::Stateless)?;
        let quote = prepared.quote();
        let result = prepared.invoke().map_err(ExecutionDriverError::Stateless)?;
        (quote, result)
    } else if invocation.contract[..19] == [0; 19] && invocation.contract[19] == 5 {
        let prepared =
            PreparedModexpCall::prepare(invocation).map_err(ExecutionDriverError::Stateless)?;
        let quote = prepared.quote();
        let result = prepared.invoke().map_err(ExecutionDriverError::Stateless)?;
        (quote, result)
    } else if invocation.contract == FALCON_VERIFY_ADDRESS {
        let prepared =
            PreparedFalconCall::prepare(invocation).map_err(ExecutionDriverError::Stateless)?;
        let quote = prepared.quote();
        let result = prepared.invoke().map_err(ExecutionDriverError::Stateless)?;
        (quote, result)
    } else if invocation.contract == P256_VERIFY_ADDRESS {
        let prepared =
            PreparedP256Call::prepare(invocation).map_err(ExecutionDriverError::Stateless)?;
        let quote = prepared.quote();
        let result = prepared.invoke().map_err(ExecutionDriverError::Stateless)?;
        (quote, result)
    } else if let Some(registry) = bls_registry(profile)
        .filter(|registry| BlsPrecompile::at_address(*registry, invocation.contract).is_some())
    {
        let prepared = PreparedBlsCall::prepare(registry, invocation)
            .map_err(ExecutionDriverError::Stateless)?;
        let quote = prepared.quote();
        let result = prepared.invoke().map_err(ExecutionDriverError::Stateless)?;
        (quote, result)
    } else {
        let prepared = PreparedCurvePrecompileCall::prepare(invocation)
            .map_err(ExecutionDriverError::Stateless)?;
        let quote = prepared.quote();
        let result = prepared.invoke().map_err(ExecutionDriverError::Stateless)?;
        (quote, result)
    };
    result
        .validate(&expected, quote)
        .map_err(ExecutionDriverError::StatelessResult)?;
    let NativeInvocationResult::Completed(outcome) = result else {
        return Ok(NativeFrameOutcome {
            status: CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas),
            required_gas: quote.required_gas,
            gas_left: expected.supplied_gas,
            output: Vec::new(),
        });
    };
    if !outcome.account_mutations.is_empty()
        || !outcome.raw_mutations.is_empty()
        || !outcome.logs.is_empty()
    {
        return Err(ExecutionDriverError::StatelessEffects {
            address: expected.contract,
        });
    }
    Ok(NativeFrameOutcome {
        status: match outcome.status {
            NativeStatus::Success => CodeExecutionStatus::Success,
            NativeStatus::ContractFailure(error) => {
                CodeExecutionStatus::Failure(CodeExecutionError::Native(error))
            }
        },
        required_gas: quote.required_gas,
        gas_left: FinalChainGas::new(expected.supplied_gas.as_u64() - quote.required_gas.as_u64()),
        output: outcome.output,
    })
}

impl std::fmt::Display for ExecutionDriverError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "execution driver: {self:?}")
    }
}

impl std::error::Error for ExecutionDriverError {}

impl From<EnvelopeError> for ExecutionDriverError {
    fn from(error: EnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

impl From<JournalError> for ExecutionDriverError {
    fn from(error: JournalError) -> Self {
        Self::Journal(error)
    }
}

impl From<NativeAdapterError> for ExecutionDriverError {
    fn from(error: NativeAdapterError) -> Self {
        Self::Native(error)
    }
}

/// Monotonic consensus-native invocation identity within one pending period.
///
/// Frame checkpoints never rewind this owner. Depth/funds pre-entry rejection
/// does not allocate an identity; quote out-of-gas and normal native failure do.
#[derive(Debug, Eq, PartialEq)]
pub struct PeriodConsensusSequence {
    period: rustaxa_types::FinalChainBlockNumber,
    next_sequence: u64,
}

impl PeriodConsensusSequence {
    /// Starts a zero-based sequence for `period`.
    #[must_use]
    pub const fn new(period: rustaxa_types::FinalChainBlockNumber) -> Self {
        Self {
            period,
            next_sequence: 0,
        }
    }

    /// Returns the pending period this sequence belongs to.
    #[must_use]
    pub const fn period(&self) -> rustaxa_types::FinalChainBlockNumber {
        self.period
    }

    /// Returns the sequence that the next reached consensus-native call receives.
    #[must_use]
    pub const fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    fn allocate(
        &mut self,
        period: rustaxa_types::FinalChainBlockNumber,
        transaction: rustaxa_types::FinalChainTransactionPosition,
    ) -> Result<NativeInvocationId, ExecutionDriverError> {
        if period != self.period {
            return Err(ExecutionDriverError::NativeSequencePeriod {
                expected: self.period,
                observed: period,
            });
        }
        let sequence = self.next_sequence;
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or(ExecutionDriverError::NativeSequenceOverflow)?;
        Ok(NativeInvocationId {
            transaction,
            sequence,
        })
    }
}

/// Executes and settles one ordinary top-level CALL.
///
/// Consensus rejections and bytecode failures are returned as transaction
/// results. Infrastructure/unsupported errors abort the pending period; the
/// caller must discard the journal because envelope admission may already have
/// changed its sender balance or nonce.
pub fn execute_top_level_call<
    R: ConcreteExecutionRead,
    B: BlockHashRead,
    N: NativeAddressClassifier,
>(
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
    native_addresses: &N,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    envelope_rules: EnvelopeRules,
    profile: TaraxaProfile,
) -> Result<TransactionExecutionResult, ExecutionDriverError> {
    if transaction.kind != ExecutionTransactionKind::Call || transaction.receiver.is_none() {
        return Err(ExecutionDriverError::UnsupportedTransactionKind(
            transaction.kind,
        ));
    }
    let admission = admit(
        journal,
        transaction,
        envelope_rules,
        IntrinsicGasSchedule::PINNED,
    )?;
    let EnvelopeAdmission::Admitted(admitted) = admission else {
        let EnvelopeAdmission::Rejected(result) = admission else {
            unreachable!()
        };
        return Ok(TransactionExecutionResult::ConsensusFailure(result));
    };
    execute_admitted_call(
        journal,
        block_hashes,
        native_addresses,
        block,
        transaction,
        &admitted,
        profile,
        None,
    )
}

/// Executes CALL with an explicit consensus port and reviewed stateless helpers.
///
/// `all_native_addresses` is the complete native/precompile set for the period;
/// `consensus_native_addresses` is the subset owned by `native_port`. An address
/// selected only by the complete set uses a reviewed stateless helper at exact
/// addresses 1–9 and profile-selected BLS, P-256 and Falcon; other such addresses
/// remain unavailable. The full classifier owns historical activation, including
/// whether Ficus enables address 9. Overlap
/// between consensus and reviewed stateless addresses is an integrity error.
/// The caller owns one [`PeriodConsensusSequence`] for the pending period and must
/// discard the journal, port and sequence together after any returned error.
#[allow(clippy::too_many_arguments)]
pub fn execute_top_level_call_with_native<
    R: ConcreteExecutionRead,
    B: BlockHashRead,
    A: NativeAddressClassifier,
    C: NativeAddressClassifier,
    P: NativeExecutionPort,
>(
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
    all_native_addresses: &A,
    consensus_native_addresses: &C,
    native_port: &mut P,
    native_sequence: &mut PeriodConsensusSequence,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    envelope_rules: EnvelopeRules,
    profile: TaraxaProfile,
) -> Result<TransactionExecutionResult, ExecutionDriverError> {
    validate_native_period(native_sequence, block.period)?;
    if transaction.kind != ExecutionTransactionKind::Call || transaction.receiver.is_none() {
        return Err(ExecutionDriverError::UnsupportedTransactionKind(
            transaction.kind,
        ));
    }
    let admission = admit(
        journal,
        transaction,
        envelope_rules,
        IntrinsicGasSchedule::PINNED,
    )?;
    let EnvelopeAdmission::Admitted(admitted) = admission else {
        let EnvelopeAdmission::Rejected(result) = admission else {
            unreachable!()
        };
        return Ok(TransactionExecutionResult::ConsensusFailure(result));
    };
    execute_admitted_call(
        journal,
        block_hashes,
        all_native_addresses,
        block,
        transaction,
        &admitted,
        profile,
        Some(NativeExecution {
            consensus_addresses: consensus_native_addresses,
            port: native_port,
            sequence: native_sequence,
            stateless_sequence: TransactionStatelessSequence::default(),
        }),
    )
}

/// Executes and settles one top-level CREATE using the exact transaction nonce.
///
/// Creator nonce increments survive collision and child failure, while the
/// child checkpoint owns target account, transfer, initcode storage/logs and
/// runtime code. Infrastructure/unsupported errors abort the pending period;
/// callers must discard the possibly admission-mutated journal.
pub fn execute_top_level_create<
    R: ConcreteExecutionRead,
    B: BlockHashRead,
    N: NativeAddressClassifier,
>(
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
    native_addresses: &N,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    envelope_rules: EnvelopeRules,
    profile: TaraxaProfile,
) -> Result<TransactionExecutionResult, ExecutionDriverError> {
    if transaction.kind != ExecutionTransactionKind::Create || transaction.receiver.is_some() {
        return Err(ExecutionDriverError::UnsupportedTransactionKind(
            transaction.kind,
        ));
    }
    let admission = admit(
        journal,
        transaction,
        envelope_rules,
        IntrinsicGasSchedule::PINNED,
    )?;
    let EnvelopeAdmission::Admitted(admitted) = admission else {
        let EnvelopeAdmission::Rejected(result) = admission else {
            unreachable!()
        };
        return Ok(TransactionExecutionResult::ConsensusFailure(result));
    };
    execute_admitted_create(
        journal,
        block_hashes,
        native_addresses,
        block,
        transaction,
        &admitted,
        profile,
        None,
    )
}

/// Executes CREATE initcode with consensus and reviewed stateless call handling.
///
/// The classifiers and period-local sequence have the same invariants as
/// [`execute_top_level_call_with_native`]. CREATE itself is ordinary; this port
/// and reviewed helpers at addresses 1–9 are available to CALL-family actions
/// yielded by its initcode descendants.
#[allow(clippy::too_many_arguments)]
pub fn execute_top_level_create_with_native<
    R: ConcreteExecutionRead,
    B: BlockHashRead,
    A: NativeAddressClassifier,
    C: NativeAddressClassifier,
    P: NativeExecutionPort,
>(
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
    all_native_addresses: &A,
    consensus_native_addresses: &C,
    native_port: &mut P,
    native_sequence: &mut PeriodConsensusSequence,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    envelope_rules: EnvelopeRules,
    profile: TaraxaProfile,
) -> Result<TransactionExecutionResult, ExecutionDriverError> {
    validate_native_period(native_sequence, block.period)?;
    if transaction.kind != ExecutionTransactionKind::Create || transaction.receiver.is_some() {
        return Err(ExecutionDriverError::UnsupportedTransactionKind(
            transaction.kind,
        ));
    }
    let admission = admit(
        journal,
        transaction,
        envelope_rules,
        IntrinsicGasSchedule::PINNED,
    )?;
    let EnvelopeAdmission::Admitted(admitted) = admission else {
        let EnvelopeAdmission::Rejected(result) = admission else {
            unreachable!()
        };
        return Ok(TransactionExecutionResult::ConsensusFailure(result));
    };
    execute_admitted_create(
        journal,
        block_hashes,
        all_native_addresses,
        block,
        transaction,
        &admitted,
        profile,
        Some(NativeExecution {
            consensus_addresses: consensus_native_addresses,
            port: native_port,
            sequence: native_sequence,
            stateless_sequence: TransactionStatelessSequence::default(),
        }),
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_admitted_create<
    R: ConcreteExecutionRead,
    B: BlockHashRead,
    N: NativeAddressClassifier,
>(
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
    native_addresses: &N,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    admitted: &AdmittedTransaction,
    profile: TaraxaProfile,
    mut native: Option<NativeExecution<'_>>,
) -> Result<TransactionExecutionResult, ExecutionDriverError> {
    let attempted = create_address(
        transaction.sender,
        &CreateScheme::Create {
            nonce: transaction.nonce.clone(),
        },
        &transaction.input,
    );
    let sender = journal.account(transaction.sender)?;
    if transaction.sender != [0_u8; 20]
        && sender.balance.value() < &BigInt::from(transaction.value.value().clone())
    {
        return settle(
            journal,
            transaction,
            admitted,
            FrameSettlement {
                status: FrameSettlementStatus::InsufficientBalanceForTransfer,
                gas_left: admitted.action_gas,
                output: Vec::new(),
                attempted_contract_address: Some(attempted),
            },
        )
        .map_err(Into::into);
    }

    journal.set_nonce(transaction.sender, transaction.nonce.next())?;
    let target = journal.account_metadata(attempted)?;
    if !target.nonce.is_zero() || target.code_size != 0 {
        return settle(
            journal,
            transaction,
            admitted,
            FrameSettlement {
                status: FrameSettlementStatus::CodeFailure(CodeExecutionError::CreateCollision),
                gas_left: FinalChainGas::ZERO,
                output: Vec::new(),
                attempted_contract_address: Some(attempted),
            },
        )
        .map_err(Into::into);
    }

    let checkpoint = journal.checkpoint();
    journal.set_nonce(attempted, rustaxa_types::FinalChainNonce::from_u64(1))?;
    journal.subtract_balance(transaction.sender, transaction.value.value())?;
    journal.add_balance(attempted, transaction.value.value())?;
    let (result, opcode) = match run_revm(
        journal,
        block_hashes,
        native_addresses,
        block,
        transaction,
        attempted,
        transaction.input.clone(),
        Vec::new(),
        admitted,
        profile,
        &mut native,
    ) {
        Ok(result) => result,
        Err(error) => {
            journal.revert_checkpoint(checkpoint)?;
            return Err(error);
        }
    };

    let mut gas_left = result.gas.remaining();
    let output = result.output.to_vec();
    let status = match map_terminal(result.result, opcode) {
        Ok(None) => {
            let deposit =
                settle_code_deposit(output.clone(), FinalChainGas::new(gas_left), 24_576, 200);
            match deposit.result {
                Ok(code) => {
                    if result.gas.refunded() < 0 {
                        journal.revert_checkpoint(checkpoint)?;
                        return Err(ExecutionDriverError::NegativeRootRefund(
                            result.gas.refunded(),
                        ));
                    }
                    gas_left = deposit.gas_remaining.as_u64();
                    journal.set_code(attempted, code)?;
                    journal.commit_checkpoint(checkpoint)?;
                    if result.gas.refunded() > 0 {
                        journal.add_refund(result.gas.refunded() as u64)?;
                    }
                    FrameSettlementStatus::Success
                }
                Err(error) => {
                    gas_left = 0;
                    journal.revert_checkpoint(checkpoint)?;
                    FrameSettlementStatus::CodeFailure(error)
                }
            }
        }
        Ok(Some(error)) => {
            journal.revert_checkpoint(checkpoint)?;
            if error != CodeExecutionError::Revert {
                gas_left = 0;
            }
            FrameSettlementStatus::CodeFailure(error)
        }
        Err(error) => {
            journal.revert_checkpoint(checkpoint)?;
            return Err(error);
        }
    };
    settle(
        journal,
        transaction,
        admitted,
        FrameSettlement {
            status,
            gas_left: FinalChainGas::new(gas_left),
            output,
            attempted_contract_address: Some(attempted),
        },
    )
    .map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
fn execute_admitted_call<R: ConcreteExecutionRead, B: BlockHashRead, N: NativeAddressClassifier>(
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
    native_addresses: &N,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    admitted: &AdmittedTransaction,
    profile: TaraxaProfile,
    mut native: Option<NativeExecution<'_>>,
) -> Result<TransactionExecutionResult, ExecutionDriverError> {
    let target = transaction.receiver.expect("call receiver checked");
    let route = classify_native(
        native_addresses,
        native.as_ref(),
        block.period,
        target,
        profile,
    )?;
    if route == NativeRoute::Unsupported {
        return Err(ExecutionDriverError::NativeCallUnavailable { address: target });
    }
    let target_metadata = journal.account_metadata(target)?;
    if route == NativeRoute::Ordinary
        && !target_metadata.exists
        && transaction.value.value() == &num_bigint::BigUint::default()
    {
        return settle(
            journal,
            transaction,
            admitted,
            FrameSettlement {
                status: FrameSettlementStatus::Success,
                gas_left: admitted.action_gas,
                output: Vec::new(),
                attempted_contract_address: None,
            },
        )
        .map_err(Into::into);
    }
    let checkpoint = journal.checkpoint();

    let sender = journal.account(transaction.sender)?;
    if transaction.sender != [0_u8; 20]
        && sender.balance.value() < &BigInt::from(transaction.value.value().clone())
    {
        journal.revert_checkpoint(checkpoint)?;
        return settle(
            journal,
            transaction,
            admitted,
            FrameSettlement {
                status: FrameSettlementStatus::InsufficientBalanceForTransfer,
                gas_left: admitted.action_gas,
                output: Vec::new(),
                attempted_contract_address: None,
            },
        )
        .map_err(Into::into);
    }
    journal.subtract_balance(transaction.sender, transaction.value.value())?;
    journal.add_balance(target, transaction.value.value())?;
    let native_outcome = match route {
        NativeRoute::Consensus => {
            let native = native
                .as_mut()
                .expect("consensus route has native execution");
            let id = match native.sequence.allocate(block.period, transaction.position) {
                Ok(id) => id,
                Err(error) => {
                    journal.revert_checkpoint(checkpoint)?;
                    return Err(error);
                }
            };
            let invocation = NativeInvocation {
                id,
                period: block.period,
                depth: 0,
                kind: NativeCallKind::Call,
                is_static: false,
                caller: transaction.sender,
                contract: target,
                state_address: target,
                value: transaction.value.clone(),
                input: transaction.input.clone(),
                supplied_gas: admitted.action_gas,
            };
            Some(invoke_native(journal, native.port, &invocation).map_err(Into::into))
        }
        NativeRoute::Stateless => {
            let id = match native
                .as_mut()
                .expect("stateless route has native execution")
                .stateless_sequence
                .allocate(transaction.position)
            {
                Ok(id) => id,
                Err(error) => {
                    journal.revert_checkpoint(checkpoint)?;
                    return Err(error);
                }
            };
            Some(invoke_stateless(
                StatelessInvocation {
                    id,
                    period: block.period,
                    depth: 0,
                    kind: NativeCallKind::Call,
                    is_static: false,
                    caller: transaction.sender,
                    contract: target,
                    state_address: target,
                    value: transaction.value.clone(),
                    input: transaction.input.clone(),
                    supplied_gas: admitted.action_gas,
                },
                profile,
            ))
        }
        NativeRoute::Ordinary | NativeRoute::Unsupported => None,
    };
    if let Some(outcome) = native_outcome {
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                journal.revert_checkpoint(checkpoint)?;
                return Err(error);
            }
        };
        let status = native_frame_status(&outcome);
        if outcome.status == CodeExecutionStatus::Success {
            journal.commit_checkpoint(checkpoint)?;
        } else {
            journal.revert_checkpoint(checkpoint)?;
        }
        return settle(
            journal,
            transaction,
            admitted,
            FrameSettlement {
                status,
                gas_left: outcome.gas_left,
                output: outcome.output,
                attempted_contract_address: None,
            },
        )
        .map_err(Into::into);
    }
    let code = journal.account_code(target)?;
    let (result, opcode) = match run_revm(
        journal,
        block_hashes,
        native_addresses,
        block,
        transaction,
        target,
        code,
        transaction.input.clone(),
        admitted,
        profile,
        &mut native,
    ) {
        Ok(result) => result,
        Err(error) => {
            journal.revert_checkpoint(checkpoint)?;
            return Err(error);
        }
    };

    let status = match map_terminal(result.result, opcode) {
        Ok(None) => {
            if result.gas.refunded() < 0 {
                journal.revert_checkpoint(checkpoint)?;
                return Err(ExecutionDriverError::NegativeRootRefund(
                    result.gas.refunded(),
                ));
            }
            journal.commit_checkpoint(checkpoint)?;
            if result.gas.refunded() > 0 {
                journal.add_refund(result.gas.refunded() as u64)?;
            }
            FrameSettlementStatus::Success
        }
        Ok(Some(error)) => {
            journal.revert_checkpoint(checkpoint)?;
            FrameSettlementStatus::CodeFailure(error)
        }
        Err(error) => {
            journal.revert_checkpoint(checkpoint)?;
            return Err(error);
        }
    };
    let gas_left = match &status {
        FrameSettlementStatus::Success
        | FrameSettlementStatus::CodeFailure(CodeExecutionError::Revert) => result.gas.remaining(),
        FrameSettlementStatus::CodeFailure(_)
        | FrameSettlementStatus::InsufficientBalanceForTransfer => 0,
    };
    settle(
        journal,
        transaction,
        admitted,
        FrameSettlement {
            status,
            gas_left: FinalChainGas::new(gas_left),
            output: result.output.to_vec(),
            attempted_contract_address: None,
        },
    )
    .map_err(Into::into)
}

#[derive(Debug)]
enum ParentInsertion {
    Root,
    Call {
        checkpoint: JournalCheckpoint,
        return_memory: std::ops::Range<usize>,
    },
    Create {
        checkpoint: JournalCheckpoint,
        attempted_address: [u8; 20],
    },
}

struct ActiveFrame {
    interpreter: Interpreter<EthInterpreter>,
    insertion: ParentInsertion,
    full_value: num_bigint::BigUint,
    last_opcode: u8,
}

#[allow(clippy::too_many_arguments)]
fn run_revm<R: ConcreteExecutionRead, B: BlockHashRead, N: NativeAddressClassifier>(
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
    native_addresses: &N,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    target: [u8; 20],
    code: Vec<u8>,
    input: Vec<u8>,
    admitted: &AdmittedTransaction,
    profile: TaraxaProfile,
    native: &mut Option<NativeExecution<'_>>,
) -> Result<(revm::interpreter::InterpreterResult, u8), ExecutionDriverError> {
    let mut interpreter = Interpreter::<EthInterpreter>::new(
        SharedMemory::new(),
        ExtBytecode::new(Bytecode::new_legacy(Bytes::from(code))),
        InputsImpl {
            target_address: Address::from(target),
            bytecode_address: Some(Address::from(target)),
            caller_address: Address::from(transaction.sender),
            input: CallInput::Bytes(Bytes::from(input)),
            call_value: U256::from_be_bytes(transaction.value.low_word()),
            depth: 0,
        },
        false,
        SpecId::ISTANBUL,
        admitted.action_gas.as_u64(),
    );
    profile.configure_interpreter(&mut interpreter);
    let mut frames = vec![ActiveFrame {
        interpreter,
        insertion: ParentInsertion::Root,
        full_value: transaction.value.value().clone(),
        last_opcode: 0,
    }];

    loop {
        let action = {
            let active = frames.last_mut().expect("root frame remains active");
            match run_until_action(
                &mut active.interpreter,
                journal,
                block_hashes,
                block,
                transaction,
                profile,
                &mut active.last_opcode,
            ) {
                Ok(action) => action,
                Err(error) => {
                    unwind_active_frames(journal, &frames)?;
                    return Err(error);
                }
            }
        };

        match action {
            InterpreterAction::Return(result) => {
                let active = frames.pop().expect("returning frame exists");
                match active.insertion {
                    ParentInsertion::Root => return Ok((result, active.last_opcode)),
                    ParentInsertion::Call {
                        checkpoint,
                        return_memory,
                    } => {
                        if let Err(error) =
                            settle_call_checkpoint(journal, checkpoint, result.result)
                        {
                            unwind_active_frames(journal, &frames)?;
                            return Err(error);
                        }
                        let parent = frames.last_mut().expect("child call has parent");
                        parent.interpreter.memory.free_child_context();
                        insert_call_result(&mut parent.interpreter, result, return_memory);
                    }
                    ParentInsertion::Create {
                        checkpoint,
                        attempted_address,
                    } => {
                        let result = match settle_create_result(
                            journal,
                            checkpoint,
                            attempted_address,
                            result,
                        ) {
                            Ok(result) => result,
                            Err(error) => {
                                unwind_active_frames(journal, &frames)?;
                                return Err(error);
                            }
                        };
                        let parent = frames.last_mut().expect("child creation has parent");
                        parent.interpreter.memory.free_child_context();
                        insert_create_result(&mut parent.interpreter, attempted_address, result);
                    }
                }
            }
            InterpreterAction::NewFrame(FrameInput::Call(inputs)) => {
                let child = match prepare_call_frame(
                    journal,
                    native_addresses,
                    block,
                    profile,
                    frames.last_mut().expect("call parent exists"),
                    *inputs,
                    transaction.position,
                    native,
                ) {
                    Ok(child) => child,
                    Err(error) => {
                        unwind_active_frames(journal, &frames)?;
                        return Err(error);
                    }
                };
                if let Some(child) = child {
                    frames.push(child);
                }
            }
            InterpreterAction::NewFrame(FrameInput::Create(inputs)) => {
                let child = match prepare_create_frame(
                    journal,
                    profile,
                    frames.last_mut().expect("creation parent exists"),
                    *inputs,
                ) {
                    Ok(child) => child,
                    Err(error) => {
                        unwind_active_frames(journal, &frames)?;
                        return Err(error);
                    }
                };
                if let Some(child) = child {
                    frames.push(child);
                }
            }
            InterpreterAction::NewFrame(FrameInput::Empty) => {
                unwind_active_frames(journal, &frames)?;
                return Err(ExecutionDriverError::PendingFrameUnavailable(
                    PendingFrameKind::Empty,
                ));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_until_action<R: ConcreteExecutionRead, B: BlockHashRead>(
    interpreter: &mut Interpreter<EthInterpreter>,
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    profile: TaraxaProfile,
    last_opcode: &mut u8,
) -> Result<InterpreterAction, ExecutionDriverError> {
    let (table, costs) = profile.execution_instruction_table::<JournalHost<'_, R, B>>();
    let mut host = JournalHost::new(
        journal,
        block_hashes,
        block,
        transaction,
        profile.gas_params(),
    );
    loop {
        *last_opcode = interpreter.bytecode.opcode();
        match interpreter.step(&table, &costs, &mut host) {
            Ok(()) => {
                if let Some(error) = host.take_error() {
                    return Err(ExecutionDriverError::Host(error));
                }
            }
            Err(result) => {
                if result == InstructionResult::FatalExternalError && *last_opcode == 0x3e {
                    return Err(ExecutionDriverError::ReferenceInstructionPanic(0x3e));
                }
                if let Some(error) = host.take_error() {
                    return Err(ExecutionDriverError::Host(error));
                }
                if interpreter.bytecode.action().is_none() {
                    interpreter.halt(result);
                }
                return Ok(interpreter.take_next_action());
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_call_frame<R: ConcreteExecutionRead, N: NativeAddressClassifier>(
    journal: &mut ExecutionJournal<R>,
    native_addresses: &N,
    block: &ExecutionBlockContext,
    profile: TaraxaProfile,
    parent: &mut ActiveFrame,
    inputs: CallInputs,
    transaction_position: rustaxa_types::FinalChainTransactionPosition,
    native: &mut Option<NativeExecution<'_>>,
) -> Result<Option<ActiveFrame>, ExecutionDriverError> {
    let input = if inputs.input.is_empty() {
        Bytes::new()
    } else {
        Bytes::copy_from_slice(&inputs.input.as_bytes_memory(&parent.interpreter.memory))
    };
    let child_depth = parent.interpreter.input.depth.saturating_add(1);
    if child_depth > 1_024 {
        insert_call_result(
            &mut parent.interpreter,
            immediate_result(InstructionResult::CallTooDeep, inputs.gas_limit),
            inputs.return_memory_offset,
        );
        return Ok(None);
    }

    let value = big_uint(inputs.value.get());
    if matches!(
        inputs.scheme,
        RevmCallScheme::Call | RevmCallScheme::CallCode
    ) && inputs.caller != Address::ZERO
        && journal.account(inputs.caller.into_array())?.balance.value()
            < &BigInt::from(value.clone())
    {
        insert_call_result(
            &mut parent.interpreter,
            immediate_result(InstructionResult::OutOfFunds, inputs.gas_limit),
            inputs.return_memory_offset,
        );
        return Ok(None);
    }

    let code_address = inputs.bytecode_address.into_array();
    let route = classify_native(
        native_addresses,
        native.as_ref(),
        block.period,
        code_address,
        profile,
    )?;
    if route == NativeRoute::Unsupported {
        return Err(ExecutionDriverError::NativeCallUnavailable {
            address: code_address,
        });
    }
    let metadata = journal.account_metadata(code_address)?;
    if route == NativeRoute::Ordinary
        && inputs.scheme == RevmCallScheme::Call
        && value == num_bigint::BigUint::default()
        && !metadata.exists
    {
        insert_call_result(
            &mut parent.interpreter,
            immediate_result(InstructionResult::Stop, inputs.gas_limit),
            inputs.return_memory_offset,
        );
        return Ok(None);
    }

    let checkpoint = journal.checkpoint();
    let mutation = match inputs.scheme {
        RevmCallScheme::Call => journal
            .subtract_balance(inputs.caller.into_array(), &value)
            .and_then(|()| journal.add_balance(inputs.target_address.into_array(), &value)),
        RevmCallScheme::StaticCall => journal.add_balance(
            inputs.target_address.into_array(),
            &num_bigint::BigUint::default(),
        ),
        RevmCallScheme::CallCode | RevmCallScheme::DelegateCall => Ok(()),
    };
    if let Err(error) = mutation {
        journal.revert_checkpoint(checkpoint)?;
        return Err(error.into());
    }

    let full_value = if inputs.scheme == RevmCallScheme::DelegateCall {
        parent.full_value.clone()
    } else {
        value
    };
    let native_outcome = match route {
        NativeRoute::Consensus => {
            let native = native
                .as_mut()
                .expect("consensus route has native execution");
            let id = match native.sequence.allocate(block.period, transaction_position) {
                Ok(id) => id,
                Err(error) => {
                    journal.revert_checkpoint(checkpoint)?;
                    return Err(error);
                }
            };
            let invocation = NativeInvocation {
                id,
                period: block.period,
                depth: u16::try_from(child_depth).expect("bounded frame depth"),
                kind: native_call_kind(inputs.scheme),
                is_static: inputs.is_static,
                caller: inputs.caller.into_array(),
                contract: code_address,
                state_address: inputs.target_address.into_array(),
                value: ExecutionValue::new(full_value.clone()),
                input: input.to_vec(),
                supplied_gas: FinalChainGas::new(inputs.gas_limit),
            };
            Some(invoke_native(journal, native.port, &invocation).map_err(Into::into))
        }
        NativeRoute::Stateless => {
            let id = match native
                .as_mut()
                .expect("stateless route has native execution")
                .stateless_sequence
                .allocate(transaction_position)
            {
                Ok(id) => id,
                Err(error) => {
                    journal.revert_checkpoint(checkpoint)?;
                    return Err(error);
                }
            };
            Some(invoke_stateless(
                StatelessInvocation {
                    id,
                    period: block.period,
                    depth: u16::try_from(child_depth).expect("bounded frame depth"),
                    kind: native_call_kind(inputs.scheme),
                    is_static: inputs.is_static,
                    caller: inputs.caller.into_array(),
                    contract: code_address,
                    state_address: inputs.target_address.into_array(),
                    value: ExecutionValue::new(full_value.clone()),
                    input: input.to_vec(),
                    supplied_gas: FinalChainGas::new(inputs.gas_limit),
                },
                profile,
            ))
        }
        NativeRoute::Ordinary | NativeRoute::Unsupported => None,
    };
    if let Some(outcome) = native_outcome {
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                journal.revert_checkpoint(checkpoint)?;
                return Err(error);
            }
        };
        if outcome.status == CodeExecutionStatus::Success {
            journal.commit_checkpoint(checkpoint)?;
        } else {
            journal.revert_checkpoint(checkpoint)?;
        }
        insert_native_call_result(
            &mut parent.interpreter,
            outcome,
            inputs.return_memory_offset,
        );
        return Ok(None);
    }

    let code = match journal.account_code(code_address) {
        Ok(code) => code,
        Err(error) => {
            journal.revert_checkpoint(checkpoint)?;
            return Err(error.into());
        }
    };
    if code.is_empty() {
        journal.commit_checkpoint(checkpoint)?;
        insert_call_result(
            &mut parent.interpreter,
            immediate_result(InstructionResult::Stop, inputs.gas_limit),
            inputs.return_memory_offset,
        );
        return Ok(None);
    }

    let child_memory = parent.interpreter.memory.new_child_context();
    let code_hash = revm::primitives::keccak256(&code);
    let mut interpreter = Interpreter::<EthInterpreter>::new(
        child_memory,
        ExtBytecode::new_with_hash(Bytecode::new_legacy(Bytes::from(code)), code_hash),
        InputsImpl {
            target_address: inputs.target_address,
            bytecode_address: Some(inputs.bytecode_address),
            caller_address: inputs.caller,
            input: CallInput::Bytes(input),
            call_value: U256::from_be_slice(&low_word_bytes(&full_value)),
            depth: child_depth,
        },
        inputs.is_static,
        SpecId::ISTANBUL,
        inputs.gas_limit,
    );
    profile.configure_interpreter(&mut interpreter);
    Ok(Some(ActiveFrame {
        interpreter,
        insertion: ParentInsertion::Call {
            checkpoint,
            return_memory: inputs.return_memory_offset,
        },
        full_value,
        last_opcode: 0,
    }))
}

fn prepare_create_frame<R: ConcreteExecutionRead>(
    journal: &mut ExecutionJournal<R>,
    profile: TaraxaProfile,
    parent: &mut ActiveFrame,
    inputs: CreateInputs,
) -> Result<Option<ActiveFrame>, ExecutionDriverError> {
    let child_depth = parent.interpreter.input.depth.saturating_add(1);
    if child_depth > 1_024 {
        insert_create_result(
            &mut parent.interpreter,
            [0_u8; 20],
            immediate_result(InstructionResult::CallTooDeep, inputs.gas_limit()),
        );
        return Ok(None);
    }

    let creator = inputs.caller().into_array();
    let value = big_uint(inputs.value());
    if creator != [0_u8; 20]
        && journal.account(creator)?.balance.value() < &BigInt::from(value.clone())
    {
        insert_create_result(
            &mut parent.interpreter,
            [0_u8; 20],
            immediate_result(InstructionResult::OutOfFunds, inputs.gas_limit()),
        );
        return Ok(None);
    }

    let nonce = journal.account(creator)?.nonce;
    let scheme = match inputs.scheme() {
        revm::context_interface::CreateScheme::Create => CreateScheme::Create {
            nonce: nonce.clone(),
        },
        revm::context_interface::CreateScheme::Create2 { salt } => CreateScheme::Create2 {
            salt: salt.to_be_bytes(),
        },
        revm::context_interface::CreateScheme::Custom { .. } => {
            return Err(ExecutionDriverError::PendingFrameUnavailable(
                PendingFrameKind::Create,
            ));
        }
    };
    let attempted_address = create_address(creator, &scheme, inputs.init_code());
    journal.set_nonce(creator, nonce.next())?;
    let target = journal.account_metadata(attempted_address)?;
    if !target.nonce.is_zero() || target.code_size != 0 {
        insert_create_result(
            &mut parent.interpreter,
            attempted_address,
            spent_result(InstructionResult::CreateCollision, inputs.gas_limit()),
        );
        return Ok(None);
    }

    let checkpoint = journal.checkpoint();
    let mutation = journal
        .set_nonce(
            attempted_address,
            rustaxa_types::FinalChainNonce::from_u64(1),
        )
        .and_then(|()| journal.subtract_balance(creator, &value))
        .and_then(|()| journal.add_balance(attempted_address, &value));
    if let Err(error) = mutation {
        journal.revert_checkpoint(checkpoint)?;
        return Err(error.into());
    }

    let child_memory = parent.interpreter.memory.new_child_context();
    let mut interpreter = Interpreter::<EthInterpreter>::new(
        child_memory,
        ExtBytecode::new(Bytecode::new_legacy(inputs.init_code().clone())),
        InputsImpl {
            target_address: Address::from(attempted_address),
            bytecode_address: None,
            caller_address: Address::from(creator),
            input: CallInput::Bytes(Bytes::new()),
            call_value: inputs.value(),
            depth: child_depth,
        },
        false,
        SpecId::ISTANBUL,
        inputs.gas_limit(),
    );
    profile.configure_interpreter(&mut interpreter);
    Ok(Some(ActiveFrame {
        interpreter,
        insertion: ParentInsertion::Create {
            checkpoint,
            attempted_address,
        },
        full_value: value,
        last_opcode: 0,
    }))
}

fn settle_call_checkpoint<R: ConcreteExecutionRead>(
    journal: &mut ExecutionJournal<R>,
    checkpoint: JournalCheckpoint,
    result: InstructionResult,
) -> Result<(), ExecutionDriverError> {
    if result.is_ok() {
        journal.commit_checkpoint(checkpoint)?;
    } else {
        journal.revert_checkpoint(checkpoint)?;
    }
    Ok(())
}

fn settle_create_result<R: ConcreteExecutionRead>(
    journal: &mut ExecutionJournal<R>,
    checkpoint: JournalCheckpoint,
    attempted_address: [u8; 20],
    mut result: InterpreterResult,
) -> Result<InterpreterResult, ExecutionDriverError> {
    if result.result.is_ok() {
        let deposit = settle_code_deposit(
            result.output.to_vec(),
            FinalChainGas::new(result.gas.remaining()),
            24_576,
            200,
        );
        match deposit.result {
            Ok(code) => {
                result.gas.set_remaining(deposit.gas_remaining.as_u64());
                if let Err(error) = journal.set_code(attempted_address, code) {
                    journal.revert_checkpoint(checkpoint)?;
                    return Err(error.into());
                }
                journal.commit_checkpoint(checkpoint)?;
            }
            Err(error) => {
                result.result = match error {
                    CodeExecutionError::ContractSize => InstructionResult::CreateContractSizeLimit,
                    CodeExecutionError::CodeDepositOutOfGas => InstructionResult::OutOfGas,
                    other => {
                        journal.revert_checkpoint(checkpoint)?;
                        return Err(ExecutionDriverError::UnsupportedTerminal(format!(
                            "unexpected code deposit error: {other:?}"
                        )));
                    }
                };
                result.gas.spend_all();
                journal.revert_checkpoint(checkpoint)?;
            }
        }
    } else {
        journal.revert_checkpoint(checkpoint)?;
    }
    Ok(result)
}

fn insert_call_result(
    parent: &mut Interpreter<EthInterpreter>,
    mut child: InterpreterResult,
    return_memory: std::ops::Range<usize>,
) {
    let result = child.result;
    let copy_len = return_memory.len().min(child.output.len());
    parent.return_data.set_buffer(child.output.clone());
    let _ = parent.stack.push(if result.is_ok() {
        U256::from(1)
    } else {
        U256::ZERO
    });
    if result.is_ok_or_revert() && copy_len != 0 {
        parent.memory.set(
            return_memory.start,
            &parent.return_data.buffer()[..copy_len],
        );
    }
    handle_reservoir_remaining_gas(result, parent.gas.tracker_mut(), child.gas.tracker_mut());
}

fn insert_native_call_result(
    parent: &mut Interpreter<EthInterpreter>,
    outcome: NativeFrameOutcome,
    return_memory: std::ops::Range<usize>,
) {
    let success = outcome.status == CodeExecutionStatus::Success;
    let copy_len = return_memory.len().min(outcome.output.len());
    parent.return_data.set_buffer(Bytes::from(outcome.output));
    let _ = parent
        .stack
        .push(if success { U256::from(1) } else { U256::ZERO });
    if success && copy_len != 0 {
        parent.memory.set(
            return_memory.start,
            &parent.return_data.buffer()[..copy_len],
        );
    }
    parent.gas.erase_cost(outcome.gas_left.as_u64());
}

const fn native_call_kind(scheme: RevmCallScheme) -> NativeCallKind {
    match scheme {
        RevmCallScheme::Call => NativeCallKind::Call,
        RevmCallScheme::CallCode => NativeCallKind::CallCode,
        RevmCallScheme::DelegateCall => NativeCallKind::DelegateCall,
        RevmCallScheme::StaticCall => NativeCallKind::StaticCall,
    }
}

fn insert_create_result(
    parent: &mut Interpreter<EthInterpreter>,
    attempted_address: [u8; 20],
    mut child: InterpreterResult,
) {
    let result = child.result;
    if result == InstructionResult::Revert {
        parent.return_data.set_buffer(child.output.clone());
    } else {
        parent.return_data.clear();
    }
    handle_reservoir_remaining_gas(result, parent.gas.tracker_mut(), child.gas.tracker_mut());
    let _ = parent.stack.push(if result.is_ok() {
        U256::from_be_slice(&attempted_address)
    } else {
        U256::ZERO
    });
}

fn immediate_result(result: InstructionResult, gas_limit: u64) -> InterpreterResult {
    InterpreterResult {
        result,
        output: Bytes::new(),
        gas: Gas::new(gas_limit),
    }
}

fn spent_result(result: InstructionResult, gas_limit: u64) -> InterpreterResult {
    let mut result = immediate_result(result, gas_limit);
    result.gas.spend_all();
    result
}

fn unwind_active_frames<R: ConcreteExecutionRead>(
    journal: &mut ExecutionJournal<R>,
    frames: &[ActiveFrame],
) -> Result<(), ExecutionDriverError> {
    for frame in frames.iter().rev() {
        match frame.insertion {
            ParentInsertion::Root => {}
            ParentInsertion::Call { checkpoint, .. }
            | ParentInsertion::Create { checkpoint, .. } => {
                journal.revert_checkpoint(checkpoint)?;
            }
        }
    }
    Ok(())
}

fn low_word_bytes(value: &num_bigint::BigUint) -> [u8; 32] {
    let bytes = value.to_bytes_be();
    let low = &bytes[bytes.len().saturating_sub(32)..];
    let mut word = [0_u8; 32];
    word[32 - low.len()..].copy_from_slice(low);
    word
}

fn big_uint(value: U256) -> num_bigint::BigUint {
    num_bigint::BigUint::from_bytes_be(&value.to_be_bytes::<32>())
}

fn map_terminal(
    result: revm::interpreter::InstructionResult,
    opcode: u8,
) -> Result<Option<CodeExecutionError>, ExecutionDriverError> {
    use revm::interpreter::InstructionResult as I;
    let mapped = match result {
        I::Stop | I::Return | I::SelfDestruct => None,
        I::Revert => Some(CodeExecutionError::Revert),
        I::InvalidOperandOOG if opcode == 0x3e => Some(CodeExecutionError::GasUintOverflow),
        I::OutOfGas
        | I::MemoryOOG
        | I::MemoryLimitOOG
        | I::PrecompileOOG
        | I::InvalidOperandOOG
        | I::ReentrancySentryOOG => Some(CodeExecutionError::OutOfGas),
        I::OpcodeNotFound | I::InvalidFEOpcode | I::NotActivated => {
            Some(CodeExecutionError::InvalidOpcode(opcode))
        }
        I::StackUnderflow => Some(CodeExecutionError::StackUnderflow),
        I::StackOverflow => Some(CodeExecutionError::StackOverflow),
        I::CallNotAllowedInsideStatic | I::StateChangeDuringStaticCall => {
            Some(CodeExecutionError::StaticViolation)
        }
        I::CallTooDeep => Some(CodeExecutionError::Depth),
        I::CreateCollision => Some(CodeExecutionError::CreateCollision),
        I::CreateContractSizeLimit => Some(CodeExecutionError::ContractSize),
        I::InvalidJump => Some(CodeExecutionError::InvalidJump),
        I::OutOfOffset => Some(CodeExecutionError::ReturnDataOutOfBounds),
        other => {
            return Err(ExecutionDriverError::UnsupportedTerminal(format!(
                "{other:?}"
            )));
        }
    };
    Ok(mapped)
}

#[cfg(test)]
mod stateless_sequence_tests {
    use super::*;

    #[test]
    fn transaction_stateless_sequence_is_zero_based_monotonic_and_checked() {
        let transaction = rustaxa_types::FinalChainTransactionPosition::new(9);
        let mut sequence = TransactionStatelessSequence::default();
        assert_eq!(
            sequence.allocate(transaction).unwrap(),
            StatelessInvocationId {
                transaction,
                ordinal: 0,
            }
        );
        assert_eq!(
            sequence.allocate(transaction).unwrap(),
            StatelessInvocationId {
                transaction,
                ordinal: 1,
            }
        );
        sequence.next_ordinal = u64::MAX;
        assert_eq!(
            sequence.allocate(transaction),
            Err(ExecutionDriverError::StatelessOrdinalOverflow)
        );
        assert_eq!(sequence.next_ordinal, u64::MAX);
    }
}
