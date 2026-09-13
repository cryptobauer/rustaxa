//! Exact JSON formatting for the pinned Go structured execution logger.
//!
//! [`serialize_structured_results`] formats already observed opcode facts. It
//! does not execute transactions, derive their outer result, replay prerequisite
//! transactions, or implement the OpenEthereum `trace` and `vmTrace` modes.

use std::{borrow::Cow, collections::BTreeMap, fmt::Write};

use revm::interpreter::InstructionResult;

use crate::trace::{TraceAddress, TraceEvent, TraceOpcode, TraceOpcodePhase, TraceWord};

/// Supplied outer result and observed events for one target transaction.
///
/// The fields correspond to the pinned Go `ExecutionResult`. `gas_used`,
/// `failed`, and `return_value` must come from the same execution that supplied
/// `events`; the serializer deliberately does not infer them from opcode rows.
#[derive(Clone, Copy, Debug)]
pub struct StructuredTraceResult<'a> {
    /// Total transaction gas reported by the execution result.
    pub gas_used: u64,
    /// Whether execution or consensus admission failed.
    pub failed: bool,
    /// Exact returned or reverted bytes, encoded without a prefix in JSON.
    pub return_value: &'a [u8],
    /// Ordered observer events for this target transaction.
    pub events: &'a [TraceEvent],
}

/// A supplied trace fact cannot be represented by the bounded Go serializer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StructuredTraceSerializationError {
    /// Go's hooked interpreter memory is word aligned; this input is not.
    UnalignedMemory {
        /// Result containing the malformed row.
        result_index: usize,
        /// Opcode-row index within that result.
        opcode_index: usize,
        /// Supplied byte length.
        length: usize,
    },
    /// The driver has not qualified this fault's Go error-object JSON shape.
    UnsupportedFault {
        /// Result containing the unsupported row.
        result_index: usize,
        /// Opcode-row index within that result.
        opcode_index: usize,
        /// Exact interpreter fault supplied by the observer.
        result: InstructionResult,
    },
}

impl std::fmt::Display for StructuredTraceSerializationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "structured trace serialization: {self:?}")
    }
}

impl std::error::Error for StructuredTraceSerializationError {}

/// Serializes target results with the pinned Go `StructLogger` JSON shape.
///
/// The output is one JSON array in the same field order as Go's
/// `ExecutionResult` and `StructLogRes`. Stack and storage words are lowercase,
/// fixed-width hex without `0x`; return bytes use lowercase variable-width hex.
/// Default capture always emits `stack`, `memory`, and `storage`, including their
/// empty forms. Qualified fault rows emit Go's concrete error-object shape `{}`.
/// Attempted storage is isolated by execution state address and reset for every
/// supplied result, matching the fresh logger created per target transaction.
///
/// Frame events produce no row because the pinned structured logger's frame
/// callbacks are no-ops. Memory must be word aligned. Only the currently
/// qualified `REVERT` and return-data-bounds faults are accepted.
pub fn serialize_structured_results(
    results: &[StructuredTraceResult<'_>],
) -> Result<Vec<u8>, StructuredTraceSerializationError> {
    let mut output = String::new();
    output.push('[');
    for (result_index, result) in results.iter().enumerate() {
        if result_index != 0 {
            output.push(',');
        }
        write!(
            output,
            "{{\"gas\":{},\"failed\":{},\"returnValue\":\"",
            result.gas_used, result.failed
        )
        .expect("writing to String cannot fail");
        push_hex(&mut output, result.return_value);
        output.push_str("\",\"structLogs\":[");

        let mut changed_values: BTreeMap<TraceAddress, BTreeMap<TraceWord, TraceWord>> =
            BTreeMap::new();
        let mut opcode_index = 0_usize;
        for event in result.events {
            let TraceEvent::Opcode(opcode) = event else {
                continue;
            };
            if opcode_index != 0 {
                output.push(',');
            }
            serialize_opcode(
                &mut output,
                &mut changed_values,
                result_index,
                opcode_index,
                opcode,
            )?;
            opcode_index += 1;
        }
        output.push_str("]}");
    }
    output.push(']');
    Ok(output.into_bytes())
}

fn serialize_opcode(
    output: &mut String,
    changed_values: &mut BTreeMap<TraceAddress, BTreeMap<TraceWord, TraceWord>>,
    result_index: usize,
    opcode_index: usize,
    opcode: &TraceOpcode,
) -> Result<(), StructuredTraceSerializationError> {
    if !opcode.memory.len().is_multiple_of(32) {
        return Err(StructuredTraceSerializationError::UnalignedMemory {
            result_index,
            opcode_index,
            length: opcode.memory.len(),
        });
    }
    let fault = match opcode.phase {
        TraceOpcodePhase::BeforeExecution => false,
        TraceOpcodePhase::Fault(InstructionResult::Revert | InstructionResult::OutOfOffset) => true,
        TraceOpcodePhase::Fault(result) => {
            return Err(StructuredTraceSerializationError::UnsupportedFault {
                result_index,
                opcode_index,
                result,
            });
        }
    };

    let address_storage = changed_values.entry(opcode.state_address).or_default();
    if let Some(write) = opcode.attempted_sstore {
        address_storage.insert(write.key, write.value);
    }

    write!(
        output,
        "{{\"pc\":{},\"op\":\"{}\",\"gas\":{},\"gasCost\":{},\"depth\":{}",
        opcode.pc,
        opcode_name(opcode.opcode),
        opcode.gas,
        opcode.gas_cost,
        opcode.depth
    )
    .expect("writing to String cannot fail");
    if fault {
        output.push_str(",\"error\":{}");
    }
    output.push_str(",\"stack\":[");
    for (index, word) in opcode.stack.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        push_quoted_hex(output, word);
    }
    output.push_str("],\"memory\":[");
    for (index, word) in opcode.memory.as_chunks::<32>().0.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        push_quoted_hex(output, word);
    }
    output.push_str("],\"storage\":{");
    for (index, (key, value)) in address_storage.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        push_quoted_hex(output, key);
        output.push(':');
        push_quoted_hex(output, value);
    }
    output.push_str("}}");
    Ok(())
}

fn push_quoted_hex(output: &mut String, bytes: &[u8]) {
    output.push('"');
    push_hex(output, bytes);
    output.push('"');
}

fn push_hex(output: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    output.reserve(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
}

fn opcode_name(opcode: u8) -> Cow<'static, str> {
    let fixed = match opcode {
        0x00 => Some("STOP"),
        0x01 => Some("ADD"),
        0x02 => Some("MUL"),
        0x03 => Some("SUB"),
        0x04 => Some("DIV"),
        0x05 => Some("SDIV"),
        0x06 => Some("MOD"),
        0x07 => Some("SMOD"),
        0x08 => Some("ADDMOD"),
        0x09 => Some("MULMOD"),
        0x0a => Some("EXP"),
        0x0b => Some("SIGNEXTEND"),
        0x10 => Some("LT"),
        0x11 => Some("GT"),
        0x12 => Some("SLT"),
        0x13 => Some("SGT"),
        0x14 => Some("EQ"),
        0x15 => Some("ISZERO"),
        0x16 => Some("AND"),
        0x17 => Some("OR"),
        0x18 => Some("XOR"),
        0x19 => Some("NOT"),
        0x1a => Some("BYTE"),
        0x1b => Some("SHL"),
        0x1c => Some("SHR"),
        0x1d => Some("SAR"),
        0x20 => Some("KECCAK256"),
        0x30 => Some("ADDRESS"),
        0x31 => Some("BALANCE"),
        0x32 => Some("ORIGIN"),
        0x33 => Some("CALLER"),
        0x34 => Some("CALLVALUE"),
        0x35 => Some("CALLDATALOAD"),
        0x36 => Some("CALLDATASIZE"),
        0x37 => Some("CALLDATACOPY"),
        0x38 => Some("CODESIZE"),
        0x39 => Some("CODECOPY"),
        0x3a => Some("GASPRICE"),
        0x3b => Some("EXTCODESIZE"),
        0x3c => Some("EXTCODECOPY"),
        0x3d => Some("RETURNDATASIZE"),
        0x3e => Some("RETURNDATACOPY"),
        0x3f => Some("EXTCODEHASH"),
        0x40 => Some("BLOCKHASH"),
        0x41 => Some("COINBASE"),
        0x42 => Some("TIMESTAMP"),
        0x43 => Some("NUMBER"),
        0x44 => Some("DIFFICULTY"),
        0x45 => Some("GASLIMIT"),
        0x46 => Some("CHAINID"),
        0x47 => Some("SELFBALANCE"),
        0x50 => Some("POP"),
        0x51 => Some("MLOAD"),
        0x52 => Some("MSTORE"),
        0x53 => Some("MSTORE8"),
        0x54 => Some("SLOAD"),
        0x55 => Some("SSTORE"),
        0x56 => Some("JUMP"),
        0x57 => Some("JUMPI"),
        0x58 => Some("PC"),
        0x59 => Some("MSIZE"),
        0x5a => Some("GAS"),
        0x5b => Some("JUMPDEST"),
        0x5c | 0xb3 => Some("TLOAD"),
        0x5d | 0xb4 => Some("TSTORE"),
        0x5e => Some("MCOPY"),
        0x5f => Some("PUSH0"),
        0xa0 => Some("LOG0"),
        0xa1 => Some("LOG1"),
        0xa2 => Some("LOG2"),
        0xa3 => Some("LOG3"),
        0xa4 => Some("LOG4"),
        0xf0 => Some("CREATE"),
        0xf1 => Some("CALL"),
        0xf2 => Some("CALLCODE"),
        0xf3 => Some("RETURN"),
        0xf4 => Some("DELEGATECALL"),
        0xf5 => Some("CREATE2"),
        0xfa => Some("STATICCALL"),
        0xfd => Some("REVERT"),
        0xfe => Some("INVALID"),
        0xff => Some("SELFDESTRUCT"),
        _ => None,
    };
    if let Some(name) = fixed {
        return Cow::Borrowed(name);
    }
    if (0x60..=0x7f).contains(&opcode) {
        return Cow::Owned(format!("PUSH{}", opcode - 0x5f));
    }
    if (0x80..=0x8f).contains(&opcode) {
        return Cow::Owned(format!("DUP{}", opcode - 0x7f));
    }
    if (0x90..=0x9f).contains(&opcode) {
        return Cow::Owned(format!("SWAP{}", opcode - 0x8f));
    }
    Cow::Owned(format!("Missing opcode 0x{opcode:x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcode_names_follow_the_pinned_table_and_fallback() {
        assert_eq!(opcode_name(0x20), "KECCAK256");
        assert_eq!(opcode_name(0x5c), "TLOAD");
        assert_eq!(opcode_name(0xb3), "TLOAD");
        assert_eq!(opcode_name(0x60), "PUSH1");
        assert_eq!(opcode_name(0x7f), "PUSH32");
        assert_eq!(opcode_name(0x80), "DUP1");
        assert_eq!(opcode_name(0x9f), "SWAP16");
        assert_eq!(opcode_name(0x0c), "Missing opcode 0xc");
    }
}
