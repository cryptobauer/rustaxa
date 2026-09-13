//! Frame-level CREATE addressing and child settlement primitives.
//!
//! These helpers preserve Taraxa's arbitrary-width creator nonce outside
//! REVM's bounded account type. They also centralize child gas, return-data and
//! code-deposit rules used by the later iterative frame driver. This module does
//! not execute bytecode or route production execution.

use revm::primitives::keccak256;
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
            let mut payload = rlp_bytes(&creator);
            payload.extend_from_slice(&rlp_bytes(&nonce.to_bytes()));
            keccak256(rlp_list(&payload))
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

fn rlp_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return bytes.to_vec();
    }
    rlp_payload(0x80, 0xb7, bytes)
}

fn rlp_list(payload: &[u8]) -> Vec<u8> {
    rlp_payload(0xc0, 0xf7, payload)
}

fn rlp_payload(short_base: u8, long_base: u8, payload: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    if payload.len() <= 55 {
        output.push(short_base + payload.len() as u8);
    } else {
        let length = minimal_usize_bytes(payload.len());
        output.push(long_base + length.len() as u8);
        output.extend_from_slice(&length);
    }
    output.extend_from_slice(payload);
    output
}

fn minimal_usize_bytes(value: usize) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    bytes[bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len() - 1)..]
        .to_vec()
}
