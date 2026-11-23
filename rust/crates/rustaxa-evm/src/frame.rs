//! Frame-level CREATE addressing and child settlement primitives.
//!
//! These helpers preserve Taraxa's arbitrary-width creator nonce outside
//! REVM's bounded account type. They also centralize child gas, return-data and
//! code-deposit rules used by the later iterative frame driver. This module does
//! not execute bytecode or route production execution.

use revm::primitives::keccak256;
use rlp::RlpStream;
use rustaxa_types::{FinalChainGas, FinalChainNonce};

use crate::contracts::CodeExecutionError;

/// CREATE or CREATE2 address derivation input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreateScheme {
    /// Legacy CREATE using the creator's exact arbitrary-width nonce.
    Create { nonce: FinalChainNonce },
    /// CREATE2 using a full salt and initcode hash.
    Create2 { salt: [u8; 32] },
}

/// Derives the attempted contract address before transfer/collision checks.
#[must_use]
pub fn create_address(creator: [u8; 20], scheme: &CreateScheme, init_code: &[u8]) -> [u8; 20] {
    let hash = match scheme {
        CreateScheme::Create { nonce } => {
            let nonce = nonce.to_bytes();
            let mut stream = RlpStream::new_list(2);
            stream.append(&creator.as_slice());
            stream.append(&nonce.as_slice());
            keccak256(stream.out())
        }
        CreateScheme::Create2 { salt } => {
            let init_hash = keccak256(init_code);
            let mut input = Vec::with_capacity(85);
            input.push(0xff);
            input.extend_from_slice(&creator);
            input.extend_from_slice(salt);
            input.extend_from_slice(init_hash.as_slice());
            keccak256(input)
        }
    };
    hash[12..].try_into().expect("Keccak address suffix")
}

/// Child-frame completion before parent-stack settlement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChildFrameStatus {
    /// Child completed successfully.
    Success,
    /// Child returned REVERT data and unused gas.
    Revert,
    /// Child halted exceptionally and consumes all supplied child gas.
    Exceptional(CodeExecutionError),
    /// Frame entry was rejected before any child gas or state was consumed.
    PreEntryRejected(ChildPreEntryError),
}

/// CREATE-family rejection before the child interpreter starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChildPreEntryError {
    /// The next frame would exceed the depth limit.
    Depth,
    /// The creator cannot transfer the requested value.
    InsufficientBalance,
}

/// Completed child facts supplied to the parent frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildFrameResult {
    /// Completion category.
    pub status: ChildFrameStatus,
    /// Gas remaining in the child on success/revert.
    pub gas_remaining: FinalChainGas,
    /// Child output or revert bytes.
    pub output: Vec<u8>,
}

/// Parent-visible result of settling a CREATE-family child.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettledCreateChild {
    /// Gas returned to the parent; exceptional halts return zero.
    pub returned_gas: FinalChainGas,
    /// REVERT bytes copied into parent return-data; other outcomes clear it.
    pub return_data: Vec<u8>,
    /// Created address pushed on success; failure pushes zero/none.
    pub created_address: Option<[u8; 20]>,
}

/// Applies CREATE-family child gas, returndata and stack-address rules.
#[must_use]
pub fn settle_create_child(
    attempted_address: [u8; 20],
    child: ChildFrameResult,
) -> SettledCreateChild {
    match child.status {
        ChildFrameStatus::Success => SettledCreateChild {
            returned_gas: child.gas_remaining,
            return_data: Vec::new(),
            created_address: Some(attempted_address),
        },
        ChildFrameStatus::Revert => SettledCreateChild {
            returned_gas: child.gas_remaining,
            return_data: child.output,
            created_address: None,
        },
        ChildFrameStatus::Exceptional(_) => SettledCreateChild {
            returned_gas: FinalChainGas::ZERO,
            return_data: Vec::new(),
            created_address: None,
        },
        ChildFrameStatus::PreEntryRejected(_) => SettledCreateChild {
            returned_gas: child.gas_remaining,
            return_data: Vec::new(),
            created_address: None,
        },
    }
}

/// Result of runtime-code size and deposit-gas validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeDepositResult {
    /// Successful runtime code or the exact frame error.
    pub result: Result<Vec<u8>, CodeExecutionError>,
    /// Child gas remaining after deposit; failures consume all gas.
    pub gas_remaining: FinalChainGas,
}

/// Validates runtime code size and charges the historical per-byte deposit gas.
#[must_use]
pub fn settle_code_deposit(
    runtime_code: Vec<u8>,
    gas_remaining: FinalChainGas,
    max_code_size: usize,
    gas_per_byte: u64,
) -> CodeDepositResult {
    if runtime_code.len() > max_code_size {
        return CodeDepositResult {
            result: Err(CodeExecutionError::ContractSize),
            gas_remaining: FinalChainGas::ZERO,
        };
    }
    let Some(cost) = (runtime_code.len() as u64).checked_mul(gas_per_byte) else {
        return CodeDepositResult {
            result: Err(CodeExecutionError::CodeDepositOutOfGas),
            gas_remaining: FinalChainGas::ZERO,
        };
    };
    let Some(remaining) = gas_remaining.checked_sub(FinalChainGas::new(cost)) else {
        return CodeDepositResult {
            result: Err(CodeExecutionError::CodeDepositOutOfGas),
            gas_remaining: FinalChainGas::ZERO,
        };
    };
    CodeDepositResult {
        result: Ok(runtime_code),
        gas_remaining: remaining,
    }
}
