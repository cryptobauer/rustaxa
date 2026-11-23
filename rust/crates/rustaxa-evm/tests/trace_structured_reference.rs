//! Checks for structured trace serialization and fault handling.

use revm::interpreter::InstructionResult;
use rustaxa_evm::{
    structured_trace::{
        StructuredTraceResult, StructuredTraceSerializationError, serialize_structured_results,
    },
    trace::{AttemptedSstore, TraceEvent, TraceOpcode, TraceOpcodePhase},
};

fn synthetic_opcode(
    address: [u8; 20],
    opcode: u8,
    stack: Vec<[u8; 32]>,
    attempted_sstore: Option<AttemptedSstore>,
) -> TraceEvent {
    TraceEvent::Opcode(TraceOpcode {
        pc: u64::from(opcode),
        opcode,
        gas: 9,
        gas_cost: 3,
        depth: 1,
        state_address: address,
        stack,
        memory: Vec::new(),
        refund: 7,
        phase: TraceOpcodePhase::BeforeExecution,
        attempted_sstore,
    })
}

#[test]
fn storage_is_address_local_and_resets_for_each_target() {
    let key = [0x11; 32];
    let value = [0x22; 32];
    let first_events = vec![
        synthetic_opcode(
            [0xaa; 20],
            0x55,
            vec![value, key],
            Some(AttemptedSstore { key, value }),
        ),
        synthetic_opcode([0xbb; 20], 0x00, Vec::new(), None),
    ];
    let second_events = vec![synthetic_opcode([0xaa; 20], 0x00, Vec::new(), None)];
    let bytes = serialize_structured_results(&[
        StructuredTraceResult {
            gas_used: 1,
            failed: false,
            return_value: &[],
            events: &first_events,
        },
        StructuredTraceResult {
            gas_used: 2,
            failed: true,
            return_value: &[0xab],
            events: &second_events,
        },
    ])
    .unwrap();
    let key_hex = "11".repeat(32);
    let value_hex = "22".repeat(32);
    let expected = format!(
        "[{{\"gas\":1,\"failed\":false,\"returnValue\":\"\",\"structLogs\":[{{\"pc\":85,\"op\":\"SSTORE\",\"gas\":9,\"gasCost\":3,\"depth\":1,\"stack\":[\"{value_hex}\",\"{key_hex}\"],\"memory\":[],\"storage\":{{\"{key_hex}\":\"{value_hex}\"}}}},{{\"pc\":0,\"op\":\"STOP\",\"gas\":9,\"gasCost\":3,\"depth\":1,\"stack\":[],\"memory\":[],\"storage\":{{}}}}]}},{{\"gas\":2,\"failed\":true,\"returnValue\":\"ab\",\"structLogs\":[{{\"pc\":0,\"op\":\"STOP\",\"gas\":9,\"gasCost\":3,\"depth\":1,\"stack\":[],\"memory\":[],\"storage\":{{}}}}]}}]"
    );
    assert_eq!(String::from_utf8(bytes).unwrap(), expected);
}

#[test]
fn fault_shape_and_unrepresentable_rows_are_explicit() {
    let mut fault = match synthetic_opcode([0; 20], 0xfd, Vec::new(), None) {
        TraceEvent::Opcode(opcode) => opcode,
        _ => unreachable!(),
    };
    fault.phase = TraceOpcodePhase::Fault(InstructionResult::Revert);
    let events = [TraceEvent::Opcode(fault.clone())];
    let bytes = serialize_structured_results(&[StructuredTraceResult {
        gas_used: 3,
        failed: true,
        return_value: &[],
        events: &events,
    }])
    .unwrap();
    assert!(
        String::from_utf8(bytes)
            .unwrap()
            .contains("\"depth\":1,\"error\":{},\"stack\":[],\"memory\":[],\"storage\":{}")
    );

    fault.memory.push(0);
    let events = [TraceEvent::Opcode(fault.clone())];
    assert_eq!(
        serialize_structured_results(&[StructuredTraceResult {
            gas_used: 3,
            failed: true,
            return_value: &[],
            events: &events,
        }]),
        Err(StructuredTraceSerializationError::UnalignedMemory {
            result_index: 0,
            opcode_index: 0,
            length: 1,
        })
    );

    fault.memory.clear();
    fault.phase = TraceOpcodePhase::Fault(InstructionResult::OutOfGas);
    let events = [TraceEvent::Opcode(fault)];
    assert_eq!(
        serialize_structured_results(&[StructuredTraceResult {
            gas_used: 3,
            failed: true,
            return_value: &[],
            events: &events,
        }]),
        Err(StructuredTraceSerializationError::UnsupportedFault {
            result_index: 0,
            opcode_index: 0,
            result: InstructionResult::OutOfGas,
        })
    );
}
