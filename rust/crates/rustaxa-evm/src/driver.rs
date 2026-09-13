//! Bounded top-level Taraxa bytecode execution through REVM.
//!
//! This driver composes the reviewed envelope, profile and journal for an
//! ordinary top-level CALL. It executes real REVM instructions and settles
//! interpreter gas/refunds once. Nested CALL/CREATE actions, native dispatch,
//! top-level creation and SELFDESTRUCT remain typed unavailable boundaries.

use num_bigint::BigInt;
use revm::{
    bytecode::Bytecode,
    interpreter::{
        CallInput, InputsImpl, Interpreter, InterpreterAction, SharedMemory,
        interpreter::{EthInterpreter, ExtBytecode},
        interpreter_types::{Jumps, LoopControl},
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
    host::{HostError, JournalHost},
    journal::{ExecutionJournal, JournalError},
    profile::TaraxaProfile,
};

/// Kind of interpreter frame request not yet implemented by this bounded driver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingFrameKind {
    /// CALL, CALLCODE, DELEGATECALL or STATICCALL needs nested-frame/native routing.
    Call,
    /// CREATE or CREATE2 needs the iterative creation driver.
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
    /// REVM yielded a frame that requires the later iterative/native driver.
    PendingFrameUnavailable(PendingFrameKind),
    /// REVM returned a terminal category that has no reviewed Taraxa mapping yet.
    UnsupportedTerminal(String),
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
pub fn execute_top_level_call<R: ConcreteStateRead, B: BlockHashRead>(
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
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
        block,
        transaction,
        &admitted,
        profile,
    )
}

fn execute_admitted_call<R: ConcreteStateRead, B: BlockHashRead>(
    journal: &mut ExecutionJournal<R>,
    block_hashes: &B,
    block: &ExecutionBlockContext,
    transaction: &ExecutionTransaction,
    admitted: &AdmittedTransaction,
    profile: TaraxaProfile,
) -> Result<TransactionExecutionResult, ExecutionDriverError> {
    let target = transaction.receiver.expect("call receiver checked");
    let checkpoint = journal.checkpoint();

    let sender = journal.account(transaction.sender)?;
    if sender.balance.value() < &BigInt::from(transaction.value.value().clone()) {
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

    let mut interpreter = Interpreter::<EthInterpreter>::new(
        SharedMemory::new(),
        ExtBytecode::new(Bytecode::new_raw(Bytes::from(code))),
        InputsImpl {
            target_address: Address::from(target),
            bytecode_address: Some(Address::from(target)),
            caller_address: Address::from(transaction.sender),
            input: CallInput::Bytes(Bytes::from(transaction.input.clone())),
            call_value: U256::from_be_bytes(transaction.value.low_word()),
            depth: 0,
        },
        false,
        SpecId::ISTANBUL,
        admitted.action_gas.as_u64(),
    );
    let (table, costs) = profile.instruction_table::<JournalHost<'_, R, B>>();
    let gas_params = profile.gas_params();
    let mut host = JournalHost::new(journal, block_hashes, block, transaction, gas_params);

    let terminal = loop {
        let opcode = interpreter.bytecode.opcode();
        match interpreter.step(&table, &costs, &mut host) {
            Ok(()) => {
                if let Some(error) = host.take_error() {
                    break Err(ExecutionDriverError::Host(error));
                }
            }
            Err(result) => {
                if let Some(error) = host.take_error() {
                    break Err(ExecutionDriverError::Host(error));
                }
                if interpreter.bytecode.action().is_none() {
                    interpreter.halt(result);
                }
                break match interpreter.take_next_action() {
                    InterpreterAction::Return(result) => Ok((result, opcode)),
                    InterpreterAction::NewFrame(frame) => {
                        let kind = match frame {
                            revm::interpreter::FrameInput::Call(_) => PendingFrameKind::Call,
                            revm::interpreter::FrameInput::Create(_) => PendingFrameKind::Create,
                            revm::interpreter::FrameInput::Empty => PendingFrameKind::Empty,
                        };
                        Err(ExecutionDriverError::PendingFrameUnavailable(kind))
                    }
                };
            }
        }
    };
    drop(host);

    let (result, opcode) = match terminal {
        Ok(result) => result,
        Err(error) => {
            journal.revert_checkpoint(checkpoint)?;
            return Err(error);
        }
    };
    let status = match map_terminal(result.result, opcode) {
        Ok(None) => {
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
