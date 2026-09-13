//! Taraxa transaction envelope admission and gas settlement.
//!
//! The envelope keeps fee, value and nonce arithmetic at the pinned Go widths
//! and leaves bytecode/native execution to the frame driver. Consensus failures
//! retain their distinct charging path and never masquerade as code failures.

use num_bigint::{BigInt, BigUint};
use rustaxa_types::FinalChainGas;

use crate::{
    contracts::{
        CodeExecutionError, CodeExecutionStatus, ConsensusFailure, ConsensusFailureResult,
        ExecutedTransactionResult, ExecutionTransaction, TransactionExecutionResult,
    },
    journal::{ExecutionJournal, JournalError},
};
use rustaxa_types::concrete_state::ConcreteStateRead;

/// Historical envelope switches selected from the finalized period.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvelopeRules {
    /// Cornus advances the supplied nonce on affordability/intrinsic failures.
    pub cornus: bool,
}

/// Gas constants used by the reference intrinsic-gas calculation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntrinsicGasSchedule {
    /// Base cost for an ordinary call transaction.
    pub transaction: FinalChainGas,
    /// Base cost for top-level contract creation.
    pub creation: FinalChainGas,
    /// Cost for one zero calldata/initcode byte.
    pub zero_byte: FinalChainGas,
    /// Cost for one nonzero calldata/initcode byte.
    pub nonzero_byte: FinalChainGas,
}

impl IntrinsicGasSchedule {
    /// Pre-Istanbul Taraxa schedule used by the pinned envelope corpus.
    pub const PINNED: Self = Self {
        transaction: FinalChainGas::new(21_000),
        creation: FinalChainGas::new(53_000),
        zero_byte: FinalChainGas::new(4),
        nonzero_byte: FinalChainGas::new(68),
    };

    /// Computes intrinsic gas with checked `u64` arithmetic.
    pub fn calculate(
        self,
        input: &[u8],
        contract_creation: bool,
    ) -> Result<FinalChainGas, ConsensusFailure> {
        let mut gas = if contract_creation {
            self.creation.as_u64()
        } else {
            self.transaction.as_u64()
        };
        for byte in input {
            gas = gas
                .checked_add(if *byte == 0 {
                    self.zero_byte.as_u64()
                } else {
                    self.nonzero_byte.as_u64()
                })
                .ok_or(ConsensusFailure::IntrinsicGasOverflow)?;
        }
        Ok(FinalChainGas::new(gas))
    }
}

/// Admitted transaction facts passed to the top-level frame driver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmittedTransaction {
    /// Gas remaining after intrinsic gas.
    pub action_gas: FinalChainGas,
    /// Full gas cap used by settlement and consensus-failure charging.
    pub gas_limit: FinalChainGas,
}

/// Admission either yields action gas or a terminal consensus result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnvelopeAdmission {
    /// Envelope checks passed and top-level execution may begin.
    Admitted(AdmittedTransaction),
    /// Envelope stopped before a normal code result was accepted.
    Rejected(ConsensusFailureResult),
}

/// Terminal top-level frame facts consumed by envelope settlement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameSettlement {
    /// Frame completion category.
    pub status: FrameSettlementStatus,
    /// Action gas left after frame execution.
    pub gas_left: FinalChainGas,
    /// Return/revert bytes retained by the reference.
    pub output: Vec<u8>,
    /// Address attempted by top-level CREATE, including failure.
    pub attempted_contract_address: Option<[u8; 20]>,
}

/// Top-level completion category before envelope gas settlement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrameSettlementStatus {
    /// Code/native execution succeeded.
    Success,
    /// Code/native execution failed after admission.
    CodeFailure(CodeExecutionError),
    /// The top-level value transfer failed; this remains a consensus outcome.
    InsufficientBalanceForTransfer,
}

/// Infrastructure failure while mutating authoritative journal state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnvelopeError {
    /// Journal read/mutation failed; the pending period must abort.
    Journal(JournalError),
    /// Frame gas exceeded the action gas admitted by the envelope.
    GasInvariant,
}

impl std::fmt::Display for EnvelopeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "transaction envelope: {self:?}")
    }
}

impl std::error::Error for EnvelopeError {}

impl From<JournalError> for EnvelopeError {
    fn from(error: JournalError) -> Self {
        Self::Journal(error)
    }
}

/// Applies up-front fee and nonce admission in reference order.
pub fn admit<R: ConcreteStateRead>(
    journal: &mut ExecutionJournal<R>,
    transaction: &ExecutionTransaction,
    rules: EnvelopeRules,
    schedule: IntrinsicGasSchedule,
) -> Result<EnvelopeAdmission, EnvelopeError> {
    let sender = transaction.sender;
    let account = journal.account(sender)?;
    let sender_nonce = account.nonce;
    let gas_cap = transaction.gas_limit.as_u64();
    let gas_fee = transaction.gas_price.value() * BigUint::from(gas_cap);
    let gas_fee_signed = BigInt::from(gas_fee.clone());

    if sender != [0_u8; 20] && account.balance.value() < &gas_fee_signed {
        let available = if transaction.gas_price.value() == &BigUint::default() {
            BigUint::default()
        } else {
            account.balance.value().to_biguint().unwrap_or_default() / transaction.gas_price.value()
        };
        let charged = &available * transaction.gas_price.value();
        journal.subtract_balance(sender, &charged)?;
        if rules.cornus && transaction.nonce >= sender_nonce {
            journal.set_nonce(sender, transaction.nonce.next())?;
        }
        return Ok(EnvelopeAdmission::Rejected(consensus_result(
            ConsensusFailure::InsufficientBalanceForGas,
            FinalChainGas::new(
                available
                    .try_into()
                    .expect("insufficient affordability quotient is below u64 gas cap"),
            ),
        )));
    }

    journal.subtract_balance(sender, &gas_fee)?;

    if transaction.nonce < sender_nonce {
        return Ok(EnvelopeAdmission::Rejected(consensus_result(
            ConsensusFailure::NonceTooLow,
            transaction.gas_limit,
        )));
    }

    let intrinsic = match schedule.calculate(&transaction.input, transaction.receiver.is_none()) {
        Ok(gas) => gas,
        Err(error) => {
            if rules.cornus {
                journal.set_nonce(sender, transaction.nonce.next())?;
            }
            return Ok(EnvelopeAdmission::Rejected(consensus_result(
                error,
                transaction.gas_limit,
            )));
        }
    };
    let Some(action_gas) = transaction.gas_limit.checked_sub(intrinsic) else {
        if rules.cornus {
            journal.set_nonce(sender, transaction.nonce.next())?;
        }
        return Ok(EnvelopeAdmission::Rejected(consensus_result(
            ConsensusFailure::IntrinsicGas,
            transaction.gas_limit,
        )));
    };

    if transaction.receiver.is_none() {
        // CREATE derives its address from this exact nonce and increments it in
        // the frame driver before collision/child checkpoint handling.
        journal.set_nonce(sender, transaction.nonce.clone())?;
    } else {
        journal.set_nonce(sender, transaction.nonce.next())?;
    }
    Ok(EnvelopeAdmission::Admitted(AdmittedTransaction {
        action_gas,
        gas_limit: transaction.gas_limit,
    }))
}

/// Settles action gas and refunds after an admitted top-level frame.
pub fn settle<R: ConcreteStateRead>(
    journal: &mut ExecutionJournal<R>,
    transaction: &ExecutionTransaction,
    admitted: &AdmittedTransaction,
    frame: FrameSettlement,
) -> Result<TransactionExecutionResult, EnvelopeError> {
    if admitted.action_gas > admitted.gas_limit || admitted.gas_limit != transaction.gas_limit {
        return Err(EnvelopeError::GasInvariant);
    }
    if frame.status == FrameSettlementStatus::InsufficientBalanceForTransfer {
        return Ok(TransactionExecutionResult::ConsensusFailure(
            ConsensusFailureResult {
                error: ConsensusFailure::InsufficientBalanceForTransfer,
                gas_used: admitted.gas_limit,
                attempted_contract_address: frame.attempted_contract_address,
                output: frame.output,
            },
        ));
    }
    if frame.gas_left > admitted.action_gas {
        return Err(EnvelopeError::GasInvariant);
    }

    let spent_before_refund = admitted
        .gas_limit
        .checked_sub(frame.gas_left)
        .unwrap_or(admitted.gas_limit);
    let refund_cap = spent_before_refund.as_u64() / 2;
    let refund = journal.refund().min(refund_cap);
    let gas_left = frame
        .gas_left
        .checked_add(FinalChainGas::new(refund))
        .expect("refund is capped by spent gas");
    let gas_used = admitted
        .gas_limit
        .checked_sub(gas_left)
        .expect("settled gas left cannot exceed gas cap");
    let refunded_fee = transaction.gas_price.value() * BigUint::from(gas_left.as_u64());
    journal.add_balance(transaction.sender, &refunded_fee)?;

    let status = match frame.status {
        FrameSettlementStatus::Success => CodeExecutionStatus::Success,
        FrameSettlementStatus::CodeFailure(error) => CodeExecutionStatus::Failure(error),
        FrameSettlementStatus::InsufficientBalanceForTransfer => unreachable!("handled above"),
    };
    Ok(TransactionExecutionResult::Executed(
        ExecutedTransactionResult {
            status,
            gas_used,
            output: frame.output,
            attempted_contract_address: frame.attempted_contract_address,
            logs: journal.logs().to_vec(),
        },
    ))
}

fn consensus_result(error: ConsensusFailure, gas_used: FinalChainGas) -> ConsensusFailureResult {
    ConsensusFailureResult {
        error,
        gas_used,
        attempted_contract_address: None,
        output: Vec::new(),
    }
}
