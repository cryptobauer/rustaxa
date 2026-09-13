//! Bounded top-level Taraxa bytecode execution through REVM.
//!
//! This driver composes the reviewed envelope, profile and journal for an
//! ordinary top-level CALL or CREATE. It executes real REVM legacy bytecode,
//! drives nested ordinary CALL/CREATE frames iteratively, and settles interpreter
//! gas/refunds once. Native dispatch and SELFDESTRUCT remain typed unavailable
//! boundaries.
//!
//! REVM loads a CALL target's known bytecode while building the frame action,
//! before this driver applies Taraxa's depth and balance pre-entry checks. A
//! missing or corrupt referenced code row can therefore surface as an
//! infrastructure error before a pre-entry rejection; complete concrete code
//! coverage is required for this bounded composition.

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
use rustaxa_types::{FinalChainGas, concrete_state::ConcreteStateRead};

use crate::{
    contracts::{
        BlockHashRead, CodeExecutionError, ExecutionBlockContext, ExecutionTransaction,
        ExecutionTransactionKind, TransactionExecutionResult,
    },
    envelope::{
        AdmittedTransaction, EnvelopeAdmission, EnvelopeError, EnvelopeRules, FrameSettlement,
        FrameSettlementStatus, IntrinsicGasSchedule, admit, settle,
    },
    frame::{CreateScheme, create_address, settle_code_deposit},
    host::{HostError, JournalHost},
    journal::{ExecutionJournal, JournalCheckpoint, JournalError},
    profile::TaraxaProfile,
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
    /// REVM yielded a malformed or explicitly unsupported frame action.
    PendingFrameUnavailable(PendingFrameKind),
    /// REVM returned a terminal category that has no reviewed Taraxa mapping yet.
    UnsupportedTerminal(String),
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

/// Executes and settles one ordinary top-level CALL.
///
/// Consensus rejections and bytecode failures are returned as transaction
/// results. Infrastructure/unsupported errors abort the pending period; the
/// caller must discard the journal because envelope admission may already have
/// changed its sender balance or nonce.
pub fn execute_top_level_call<
    R: ConcreteStateRead,
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
    )
}

/// Executes and settles one top-level CREATE using the exact transaction nonce.
///
/// Creator nonce increments survive collision and child failure, while the
/// child checkpoint owns target account, transfer, initcode storage/logs and
/// runtime code. Infrastructure/unsupported errors abort the pending period;
/// callers must discard the possibly admission-mutated journal.
pub fn execute_top_level_create<
    R: ConcreteStateRead,
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
    )
}

fn execute_admitted_create<R: ConcreteStateRead, B: BlockHashRead, N: NativeAddressClassifier>(
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
    native_addresses: &N,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    admitted: &AdmittedTransaction,
    profile: TaraxaProfile,
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

fn execute_admitted_call<R: ConcreteStateRead, B: BlockHashRead, N: NativeAddressClassifier>(
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
    native_addresses: &N,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    admitted: &AdmittedTransaction,
    profile: TaraxaProfile,
) -> Result<TransactionExecutionResult, ExecutionDriverError> {
    let target = transaction.receiver.expect("call receiver checked");
    if native_addresses.is_native_address(block.period, target) {
        return Err(ExecutionDriverError::NativeCallUnavailable { address: target });
    }
    let target_metadata = journal.account_metadata(target)?;
    if !target_metadata.exists && transaction.value.value() == &num_bigint::BigUint::default() {
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
fn run_revm<R: ConcreteStateRead, B: BlockHashRead, N: NativeAddressClassifier>(
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
fn run_until_action<R: ConcreteStateRead, B: BlockHashRead>(
    interpreter: &mut Interpreter<EthInterpreter>,
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    profile: TaraxaProfile,
    last_opcode: &mut u8,
) -> Result<InterpreterAction, ExecutionDriverError> {
    let (table, costs) = profile.instruction_table::<JournalHost<'_, R, B>>();
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

fn prepare_call_frame<R: ConcreteStateRead, N: NativeAddressClassifier>(
    journal: &mut ExecutionJournal<R>,
    native_addresses: &N,
    block: &ExecutionBlockContext,
    profile: TaraxaProfile,
    parent: &mut ActiveFrame,
    inputs: CallInputs,
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
    if native_addresses.is_native_address(block.period, code_address) {
        return Err(ExecutionDriverError::NativeCallUnavailable {
            address: code_address,
        });
    }
    let metadata = journal.account_metadata(code_address)?;
    if inputs.scheme == RevmCallScheme::Call
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

    let (code_hash, code) = inputs.known_bytecode;
    if code.is_empty() {
        journal.commit_checkpoint(checkpoint)?;
        insert_call_result(
            &mut parent.interpreter,
            immediate_result(InstructionResult::Stop, inputs.gas_limit),
            inputs.return_memory_offset,
        );
        return Ok(None);
    }

    let full_value = if inputs.scheme == RevmCallScheme::DelegateCall {
        parent.full_value.clone()
    } else {
        value
    };
    let child_memory = parent.interpreter.memory.new_child_context();
    let mut interpreter = Interpreter::<EthInterpreter>::new(
        child_memory,
        ExtBytecode::new_with_hash(code, code_hash),
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

fn prepare_create_frame<R: ConcreteStateRead>(
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

fn settle_call_checkpoint<R: ConcreteStateRead>(
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

fn settle_create_result<R: ConcreteStateRead>(
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

fn unwind_active_frames<R: ConcreteStateRead>(
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
        I::Stop | I::Return => None,
        I::Revert => Some(CodeExecutionError::Revert),
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
        other => {
            return Err(ExecutionDriverError::UnsupportedTerminal(format!(
                "{other:?}"
            )));
        }
    };
    Ok(mapped)
}
